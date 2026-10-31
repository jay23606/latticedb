//! Reconnect supervisor tests: each class of end, the window reset (only on a delivered event), the
//! failure bound, stickiness of stops, and generated input sequences agreeing with a model.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use lattice_sdk::backoff::Backoff;
use lattice_sdk::client::{ClientError, CloseReason, Phase, WireError};
use lattice_sdk::reconnect::{Decision, Input, StopReason, Supervisor};
use proptest::prelude::*;

fn supervisor(base_ms: u64, cap_ms: u64, max_failures: Option<u32>) -> Supervisor {
    Supervisor::new(Backoff::new(base_ms, cap_ms).unwrap(), max_failures)
}

/// Feeds `input` with the largest jitter, so a retry waits for the whole window.
fn step(s: &mut Supervisor, input: Input) -> Decision {
    s.on_input(input, u64::MAX)
}

fn server_error(code: WireError) -> Input {
    Input::Closed(CloseReason::ServerError {
        code,
        message: String::from("x"),
    })
}

fn retry_list(delays: &[u64]) -> Vec<Decision> {
    delays.iter().copied().map(Decision::RetryAfter).collect()
}

#[test]
fn fatal_server_and_client_ends_stop_with_their_reason() {
    let cases = [
        (
            Input::Closed(CloseReason::Incompatible),
            StopReason::Incompatible,
        ),
        (
            server_error(WireError::UnsupportedVersion),
            StopReason::Incompatible,
        ),
        (Input::Unauthorized, StopReason::Unauthorized),
        (
            Input::Closed(CloseReason::ByApplication),
            StopReason::ByApplication,
        ),
        (
            server_error(WireError::InvalidCursor),
            StopReason::Permanent(WireError::InvalidCursor),
        ),
        (
            server_error(WireError::InvalidScope),
            StopReason::Permanent(WireError::InvalidScope),
        ),
        (
            server_error(WireError::Malformed),
            StopReason::Permanent(WireError::Malformed),
        ),
        (
            server_error(WireError::TooManySubscriptions),
            StopReason::Permanent(WireError::TooManySubscriptions),
        ),
        (
            server_error(WireError::UnknownSubscription),
            StopReason::Permanent(WireError::UnknownSubscription),
        ),
    ];
    for (input, reason) in cases {
        let mut s = supervisor(100, 1_000, None);
        assert_eq!(
            step(&mut s, input.clone()),
            Decision::Stop(reason),
            "{input:?}"
        );
    }
}

#[test]
fn every_refused_call_stops_as_call_refused() {
    let errors = [
        ClientError::InvalidConfig("bad"),
        ClientError::InvalidScope,
        ClientError::InvalidCursor,
        ClientError::WrongPhase(Phase::Ready),
        ClientError::UnknownSubscription(3),
        ClientError::AckBeyondDelivered,
        ClientError::SubscriptionIdsExhausted,
    ];
    for error in errors {
        let mut s = supervisor(100, 1_000, None);
        assert_eq!(
            step(&mut s, Input::Failed(error.clone())),
            Decision::Stop(StopReason::CallRefused),
            "{error:?}"
        );
    }
}

#[test]
fn transient_ends_retry_with_backoff() {
    let transient = [
        Input::TransportClosed,
        Input::Closed(CloseReason::PingTimeout),
        Input::Closed(CloseReason::HandshakeTimeout),
        server_error(WireError::Internal),
        Input::Closed(CloseReason::ProtocolViolation("events before welcome")),
    ];
    for input in transient {
        let mut s = supervisor(100, 1_000, None);
        assert_eq!(
            step(&mut s, input.clone()),
            Decision::RetryAfter(100),
            "{input:?}"
        );
    }
}

#[test]
fn welcome_then_drop_keeps_growing_the_delay() {
    // The server answers the handshake and then drops the connection before any event arrives.
    let mut s = supervisor(100, 1_000, None);
    let mut delays = Vec::new();
    for _ in 0..6 {
        assert_eq!(step(&mut s, Input::Connected), Decision::Continue);
        assert_eq!(step(&mut s, Input::FrameReceived), Decision::Continue);
        delays.push(step(&mut s, Input::TransportClosed));
    }
    assert_eq!(delays, retry_list(&[100, 200, 400, 800, 1_000, 1_000]));
}

#[test]
fn a_frame_alone_does_not_reset_the_window() {
    let mut s = supervisor(100, 60_000, None);
    for _ in 0..3 {
        let _ = step(&mut s, Input::TransportClosed);
    }
    assert_eq!(step(&mut s, Input::FrameReceived), Decision::Continue);
    // The window after three failures is 800; a frame must not bring it back to 100.
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(800)
    );
}

#[test]
fn a_delivered_event_returns_the_window_to_its_first_ceiling() {
    let mut s = supervisor(100, 60_000, None);
    for _ in 0..3 {
        let _ = step(&mut s, Input::TransportClosed);
    }
    assert_eq!(step(&mut s, Input::Connected), Decision::Continue);
    assert_eq!(step(&mut s, Input::FrameReceived), Decision::Continue);
    assert_eq!(step(&mut s, Input::EventDelivered), Decision::Continue);
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(100)
    );
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(200)
    );
}

#[test]
fn a_zero_delay_still_advances_the_window() {
    let mut s = supervisor(100, 1_000, None);
    assert_eq!(
        s.on_input(Input::TransportClosed, 0),
        Decision::RetryAfter(0)
    );
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(200)
    );
}

#[test]
fn jitter_scales_the_delay_inside_the_window() {
    let mut s = supervisor(100, 1_000, None);
    // (100 + 1) * 2^63 / 2^64 = 50.5, rounded down.
    assert_eq!(
        s.on_input(Input::TransportClosed, 1 << 63),
        Decision::RetryAfter(50)
    );
}

#[test]
fn a_long_storm_of_failures_stays_at_the_cap() {
    let mut s = supervisor(1, 5_000, None);
    let mut last = Decision::Continue;
    for _ in 0..300 {
        last = step(&mut s, Input::TransportClosed);
        if let Decision::RetryAfter(d) = last {
            assert!(d <= 5_000);
        }
    }
    assert_eq!(last, Decision::RetryAfter(5_000));
}

#[test]
fn unbounded_retries_never_give_up() {
    let mut s = supervisor(1, 10, None);
    for _ in 0..500 {
        assert!(matches!(
            step(&mut s, Input::TransportClosed),
            Decision::RetryAfter(_)
        ));
    }
}

#[test]
fn bounded_retries_give_up_at_the_bound_and_stay_stopped() {
    let mut s = supervisor(100, 1_000, Some(3));
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(100)
    );
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(200)
    );
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::Stop(StopReason::GaveUp)
    );
    assert_eq!(
        step(&mut s, Input::EventDelivered),
        Decision::Stop(StopReason::GaveUp)
    );
    assert_eq!(
        step(&mut s, Input::Connected),
        Decision::Stop(StopReason::GaveUp)
    );
}

#[test]
fn a_bound_of_zero_or_one_gives_up_at_the_first_failure() {
    for bound in [0, 1] {
        let mut s = supervisor(100, 1_000, Some(bound));
        assert_eq!(
            step(&mut s, Input::TransportClosed),
            Decision::Stop(StopReason::GaveUp),
            "bound {bound}"
        );
    }
}

#[test]
fn a_delivered_event_clears_the_failure_count() {
    let mut s = supervisor(100, 1_000, Some(3));
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(100)
    );
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(200)
    );
    assert_eq!(step(&mut s, Input::EventDelivered), Decision::Continue);
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(100)
    );
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(200)
    );
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::Stop(StopReason::GaveUp)
    );
}

#[test]
fn a_frame_does_not_clear_the_failure_count() {
    let mut s = supervisor(100, 1_000, Some(2));
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(100)
    );
    assert_eq!(step(&mut s, Input::Connected), Decision::Continue);
    assert_eq!(step(&mut s, Input::FrameReceived), Decision::Continue);
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::Stop(StopReason::GaveUp)
    );
}

#[test]
fn a_fatal_end_after_transient_failures_stops_at_once() {
    let mut s = supervisor(100, 1_000, Some(10));
    assert_eq!(
        step(&mut s, Input::TransportClosed),
        Decision::RetryAfter(100)
    );
    assert_eq!(
        step(&mut s, Input::Unauthorized),
        Decision::Stop(StopReason::Unauthorized)
    );
}

#[test]
fn stop_is_final_even_after_a_frame_an_event_or_a_new_connection() {
    let mut s = supervisor(100, 1_000, None);
    assert_eq!(
        step(&mut s, Input::Unauthorized),
        Decision::Stop(StopReason::Unauthorized)
    );
    for input in [
        Input::Connected,
        Input::FrameReceived,
        Input::EventDelivered,
        Input::TransportClosed,
    ] {
        assert_eq!(
            step(&mut s, input.clone()),
            Decision::Stop(StopReason::Unauthorized),
            "{input:?}"
        );
    }
}

/// The window for `attempt` (zero based), written out from the definition rather than from the code:
/// `min(cap, base * 2^attempt)`, with the product computed wide enough that it cannot overflow.
fn window_model(base: u64, cap: u64, attempt: u32) -> u64 {
    if attempt >= 64 {
        return cap;
    }
    let product = u128::from(base) << attempt;
    u64::try_from(product).map_or(cap, |p| p.min(cap))
}

/// How the model treats an input, written from the policy table.
#[derive(Clone, Copy)]
enum Class {
    Neutral,
    Healthy,
    Transient,
    Fatal(StopReason),
}

fn model_class(input: &Input) -> Class {
    match input {
        Input::Connected | Input::FrameReceived => Class::Neutral,
        Input::EventDelivered => Class::Healthy,
        Input::TransportClosed
        | Input::Closed(CloseReason::PingTimeout | CloseReason::HandshakeTimeout) => {
            Class::Transient
        }
        Input::Closed(CloseReason::ProtocolViolation(_)) => Class::Transient,
        Input::Closed(CloseReason::ServerError {
            code: WireError::Internal,
            ..
        }) => Class::Transient,
        Input::Closed(CloseReason::Incompatible) => Class::Fatal(StopReason::Incompatible),
        Input::Closed(CloseReason::ServerError {
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

fn input_strategy() -> impl Strategy<Value = Input> {
    prop_oneof![
        Just(Input::Connected),
        Just(Input::FrameReceived),
        Just(Input::EventDelivered),
        Just(Input::TransportClosed),
        Just(Input::Unauthorized),
        Just(Input::Failed(ClientError::InvalidCursor)),
        Just(Input::Failed(ClientError::WrongPhase(Phase::Ready))),
        Just(Input::Closed(CloseReason::Incompatible)),
        Just(Input::Closed(CloseReason::ByApplication)),
        Just(Input::Closed(CloseReason::PingTimeout)),
        Just(Input::Closed(CloseReason::HandshakeTimeout)),
        Just(Input::Closed(CloseReason::ProtocolViolation("bad frame"))),
        Just(server_error(WireError::Internal)),
        Just(server_error(WireError::UnsupportedVersion)),
        Just(server_error(WireError::InvalidCursor)),
        Just(server_error(WireError::Malformed)),
    ]
}

proptest! {
    #[test]
    fn generated_sequences_agree_with_the_model(
        base in 1u64..=10_000,
        extra in 0u64..=1_000_000,
        max_failures in proptest::option::of(0u32..=6),
        steps in proptest::collection::vec((input_strategy(), any::<u64>()), 0..150),
    ) {
        let cap = base + extra;
        let mut s = Supervisor::new(Backoff::new(base, cap).unwrap(), max_failures);
        let mut attempt: u32 = 0;
        let mut failures: u32 = 0;
        let mut stopped: Option<StopReason> = None;
        for (input, jitter) in steps {
            let expected = if let Some(reason) = stopped {
                Decision::Stop(reason)
            } else {
                match model_class(&input) {
                    Class::Neutral => Decision::Continue,
                    Class::Healthy => {
                        attempt = 0;
                        failures = 0;
                        Decision::Continue
                    }
                    Class::Fatal(reason) => {
                        stopped = Some(reason);
                        Decision::Stop(reason)
                    }
                    Class::Transient => {
                        failures = failures.saturating_add(1);
                        if max_failures.is_some_and(|max| failures >= max) {
                            stopped = Some(StopReason::GaveUp);
                            Decision::Stop(StopReason::GaveUp)
                        } else {
                            let ceiling = window_model(base, cap, attempt);
                            attempt = attempt.saturating_add(1);
                            // Uniform over 0..=ceiling: (ceiling + 1) * jitter / 2^64, rounded down.
                            let wide = ((u128::from(ceiling) + 1) * u128::from(jitter)) >> 64;
                            Decision::RetryAfter(u64::try_from(wide).unwrap_or(ceiling))
                        }
                    }
                }
            };
            let got = s.on_input(input, jitter);
            prop_assert_eq!(got, expected);
            if let Decision::RetryAfter(d) = got {
                prop_assert!(d <= cap);
            }
        }
    }
}
