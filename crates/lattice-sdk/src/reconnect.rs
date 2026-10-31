//! Reconnect supervision: after a connection ends, decide whether to retry, how long to wait, or to stop.
//!
//! [`Supervisor`] is a pure decision function in the style of [`crate::backoff::Backoff`] and
//! [`crate::client::SyncClient`]. It holds no socket, reads no clock and draws no randomness. The
//! caller reports each thing that happens as an [`Input`], in the order it happened, and passes a
//! uniform `u64` sample as jitter. The caller then carries out the [`Decision`]: for
//! [`Decision::RetryAfter`] it waits that many milliseconds on its own clock, opens a new transport
//! and calls [`crate::client::SyncClient::reconnect`]. For [`Decision::Stop`] it does not reconnect.
//!
//! # Policy
//!
//! | Input | Class | Effect |
//! |---|---|---|
//! | `Connected`, `FrameReceived` | healthy so far | none; `FrameReceived` does not reset the backoff |
//! | `EventDelivered` | healthy | backoff returns to its first window, failure count returns to zero |
//! | `TransportClosed` | transient | retry with backoff |
//! | `Closed(PingTimeout)`, `Closed(HandshakeTimeout)` | transient | retry with backoff |
//! | `Closed(ServerError { Internal })` | transient (server failure) | retry with backoff |
//! | `Closed(ProtocolViolation)` | transient, bounded | retry with backoff; may give up (see below) |
//! | `Closed(Incompatible)`, `Closed(ServerError { UnsupportedVersion })` | fatal | `Stop(Incompatible)` |
//! | `Unauthorized` | fatal | `Stop(Unauthorized)` |
//! | `Closed(ByApplication)` | fatal | `Stop(ByApplication)` |
//! | `Closed(ServerError { code })` for every other code (`InvalidCursor`, `InvalidScope`, `Malformed`, `TooManySubscriptions`, `UnknownSubscription`) | fatal | `Stop(Permanent(code))` |
//! | `Failed(_)` (any [`ClientError`]) | fatal | `Stop(CallRefused)` |
//!
//! A fatal class repeats on every new connection, because the client, its credentials or its saved
//! cursor stay the same. Stopping is final: every later input gets the same `Stop`.
//!
//! `ProtocolViolation` is a judgement call: a server that breaks the protocol may be mid-deploy, so
//! it is retried, but it counts toward the bound below, so it cannot loop forever when `max_failures`
//! is set.
//!
//! Transient failures are bounded when the supervisor is given `max_failures`: the failure that
//! brings the count in a row to that bound stops the supervisor with [`StopReason::GaveUp`]. The count
//! is over transient failures with no [`Input::EventDelivered`] in between; a frame does not clear it.
//! Without a bound, transient failures retry forever, each wait at most the cap.
//!
//! The backoff window grows on every transient failure. It returns to its first window only when an
//! event is delivered to the application ([`Input::EventDelivered`]). A server that sends
//! `Welcome` and then drops the connection is therefore retried at a growing interval, not near the
//! base delay.

use crate::backoff::Backoff;
use crate::client::{ClientError, CloseReason, WireError};

/// What happened to the connection, as the caller observed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// A new transport is up and the client's `Hello` was sent. This does not reset anything.
    Connected,
    /// A frame arrived and the client accepted it, for example `Welcome` or `Pong`. This does not
    /// reset the backoff: a server that answers the handshake and then drops is still failing.
    FrameReceived,
    /// The client returned [`crate::client::Action::Deliver`]: the application got an event. This is
    /// the only sign of health that resets the backoff and the failure count.
    EventDelivered,
    /// The transport ended with no protocol reason: end of stream, a reset or a socket timeout.
    TransportClosed,
    /// The client ended the connection with a reason, from [`crate::client::Action::Close`].
    Closed(CloseReason),
    /// The transport refused the credentials before the protocol started, for example HTTP 401 or
    /// 403 on the WebSocket upgrade. The sync protocol has no frame for this, so the transport reports it.
    Unauthorized,
    /// A call on the client was refused, for example [`crate::client::SyncClient::subscribe`] with
    /// an invalid scope, or [`crate::client::SyncClient::reconnect`] in the wrong phase.
    Failed(ClientError),
}

/// What the caller does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to do: the connection is up, or was just opened.
    Continue,
    /// Wait this many milliseconds, then open a new transport and call
    /// [`crate::client::SyncClient::reconnect`]. Zero is possible (a jitter of 0): the caller adds a
    /// minimum wait if it wants one.
    RetryAfter(u64),
    /// Do not reconnect. Final: every later input gets the same decision.
    Stop(StopReason),
}

/// Why the supervisor stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The server speaks no version this client speaks.
    Incompatible,
    /// The transport refused the credentials ([`Input::Unauthorized`]).
    Unauthorized,
    /// The application closed the client ([`CloseReason::ByApplication`]).
    ByApplication,
    /// The server sent a connection-level error code that a new connection would repeat (for
    /// example an invalid cursor or scope, or a malformed frame from this client).
    Permanent(WireError),
    /// A client call was refused on its own inputs or state ([`Input::Failed`]); a new connection
    /// would repeat it.
    CallRefused,
    /// The configured number of transient failures came in a row with no delivered event.
    GaveUp,
}

/// Reconnect decisions over a [`Backoff`]. See the module documentation for the policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Supervisor {
    backoff: Backoff,
    max_failures: Option<u32>,
    failures: u32,
    stopped: Option<StopReason>,
}

/// How an input is treated. Private: the public contract is the policy table in the module docs.
enum Class {
    Healthy,
    Neutral,
    Transient,
    Fatal(StopReason),
}

fn classify(input: &Input) -> Class {
    match input {
        Input::Connected | Input::FrameReceived => Class::Neutral,
        Input::EventDelivered => Class::Healthy,
        Input::TransportClosed
        | Input::Closed(
            CloseReason::PingTimeout
            | CloseReason::HandshakeTimeout
            | CloseReason::ProtocolViolation(_)
            | CloseReason::ServerError {
                code: WireError::Internal,
                ..
            },
        ) => Class::Transient,
        Input::Closed(CloseReason::Incompatible)
        | Input::Closed(CloseReason::ServerError {
            code: WireError::UnsupportedVersion,
            ..
        }) => Class::Fatal(StopReason::Incompatible),
        Input::Closed(CloseReason::ByApplication) => Class::Fatal(StopReason::ByApplication),
        Input::Closed(CloseReason::ServerError { code, .. }) => {
            Class::Fatal(StopReason::Permanent(*code))
        }
        Input::Unauthorized => Class::Fatal(StopReason::Unauthorized),
        Input::Failed(_) => Class::Fatal(StopReason::CallRefused),
    }
}

impl Supervisor {
    /// A supervisor that starts at the first window of `backoff`. With `Some(n)`, the transient
    /// failure that brings the count in a row (no delivered event between them) to `n` stops the
    /// supervisor with [`StopReason::GaveUp`]; `Some(0)` and `Some(1)` give up at the first failure.
    /// `None` retries forever.
    pub fn new(backoff: Backoff, max_failures: Option<u32>) -> Self {
        Self {
            backoff,
            max_failures,
            failures: 0,
            stopped: None,
        }
    }

    /// The decision after `input`. `jitter` is a uniform sample over `u64`, as for
    /// [`Backoff::next_delay`]; it is used only when a retry is scheduled.
    #[must_use]
    pub fn on_input(&mut self, input: Input, jitter: u64) -> Decision {
        if let Some(reason) = self.stopped {
            return Decision::Stop(reason);
        }
        match classify(&input) {
            Class::Neutral => Decision::Continue,
            Class::Healthy => {
                self.backoff.reset();
                self.failures = 0;
                Decision::Continue
            }
            Class::Fatal(reason) => {
                self.stopped = Some(reason);
                Decision::Stop(reason)
            }
            Class::Transient => {
                self.failures = self.failures.saturating_add(1);
                if self.max_failures.is_some_and(|max| self.failures >= max) {
                    self.stopped = Some(StopReason::GaveUp);
                    return Decision::Stop(StopReason::GaveUp);
                }
                Decision::RetryAfter(self.backoff.next_delay(jitter))
            }
        }
    }
}
