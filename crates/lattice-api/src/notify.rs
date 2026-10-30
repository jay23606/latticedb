//! A commit notifier: "something committed up to LSN `x`", with no payload.
//!
//! Subscribers used to learn about a commit only by polling [`crate::feed_source::FeedSource`].
//! The database thread (the executor actor) now publishes the newest committed LSN here after every
//! statement it runs, and a waiter sleeps until that LSN passes the one it has already handled. The
//! timeout of the wait is the fallback: a waiter that is told nothing still polls when it expires,
//! so a missed wake costs latency, never an event.
//!
//! Design, in the order of the properties it has to keep:
//!
//! * **Never lost.** There is one value, the newest published LSN, and a waiter compares it with
//!   the LSN it has seen while holding the same lock the publisher takes. A commit published before
//!   the waiter looked makes the wait return at once; one published after the waiter went to sleep
//!   finds it on the condition variable. There is no window in between, and a spurious wake only
//!   makes the waiter look again. The LSN doubles as the version counter because it only grows.
//! * **Bounded.** A waiter stores nothing: its "seen" LSN is an argument held by the caller. The
//!   notifier holds one `u64`, a flag and a count of sleepers. At most
//!   [`HostLimits::commit_waiters`](crate::exec_host::HostLimits::commit_waiters) threads may sleep
//!   at once; the next one gets [`WaitError::Full`] and falls back to polling. Publishing is one
//!   lock and one `notify_all`, so a commit does not get slower with the number of waiters it wakes,
//!   and it never waits for a waiter (a sleeping waiter has released the lock).
//! * **No clock.** This crate may not read a clock, so a wait is a `Condvar::wait_timeout` whose
//!   remaining time drops to zero when a wait times out, the same rule as the budget in
//!   `exec_host`. A wake that is not a timeout costs nothing, so a waiter whose LSN is ahead of the
//!   database can outlast its timeout while commits keep arriving; each such wake is progress by
//!   the actor.
//! * **Fails closed.** `CommitNotifier::close` (shutdown, or the actor ending for any reason,
//!   including a panic) wakes every waiter with [`WaitError::Closed`], and later waits return it at
//!   once, even when a commit is pending.
//! * **Carries no data.** The value is the LSN and nothing else; no row, table or scope name. What a
//!   woken waiter may read is decided where it always was, by the feed source for its principal.
//!   The wake itself reveals that the database committed something up to that LSN, to every holder
//!   of a [`CommitNotifier`] whatever their grants (threat model G-07, issue #363); a gateway should
//!   hand it only to authenticated connections and may coarsen it (for example by treating a poll
//!   that finds nothing readable as normal and not reporting the wake to the client).

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use lattice_core::bugs::{self, Bug};

/// Sleepers allowed at once when the host is not given another number.
pub const DEFAULT_MAX_WAITERS: usize = 1024;

/// Why a wait ended without a newer LSN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitError {
    /// The host is shutting down or its database thread ended. Nothing more will be published.
    Closed,
    /// The wait ran out with nothing newer published. Poll anyway: this is the fallback.
    TimedOut,
    /// The sleeper limit is reached. Poll on a timer instead of waiting.
    Full,
}

struct State {
    /// The newest LSN published; rises only.
    latest: u64,
    closed: bool,
    sleepers: usize,
}

/// The newest committed LSN, shared between the database thread (which publishes) and any number of
/// waiting threads. See the module comment for the guarantees.
pub struct CommitNotifier {
    state: Mutex<State>,
    changed: Condvar,
    max_waiters: usize,
}

impl Default for CommitNotifier {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_WAITERS)
    }
}

fn locked(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl CommitNotifier {
    /// A notifier that lets at most `max_waiters` threads sleep at once (raised to 1).
    pub fn new(max_waiters: usize) -> Self {
        CommitNotifier {
            state: Mutex::new(State {
                latest: 0,
                closed: false,
                sleepers: 0,
            }),
            changed: Condvar::new(),
            max_waiters: max_waiters.max(1),
        }
    }

    /// Records that everything up to `lsn` is committed and wakes the sleepers. An LSN that is not
    /// newer than the current one changes nothing, so the value never goes back.
    pub(crate) fn publish(&self, lsn: u64) {
        let mut state = locked(&self.state);
        if lsn > state.latest {
            state.latest = lsn;
            self.changed.notify_all();
        }
    }

    /// Ends the notifier: sleepers wake with [`WaitError::Closed`] and so does every later wait.
    pub(crate) fn close(&self) {
        let mut state = locked(&self.state);
        state.closed = true;
        self.changed.notify_all();
    }

    /// The newest published LSN (0 before any commit).
    pub fn latest(&self) -> u64 {
        locked(&self.state).latest
    }

    /// Threads asleep in [`wait_after`](Self::wait_after) now.
    pub fn sleepers(&self) -> usize {
        locked(&self.state).sleepers
    }

    /// Returns the newest published LSN as soon as it is greater than `seen`, which is at once if it
    /// already is. Otherwise sleeps for at most `timeout` (counted as described in the module
    /// comment) and then reports [`WaitError::TimedOut`]. The caller keeps `seen`; pass the LSN it
    /// last handled, or 0 for "anything".
    ///
    /// A returned LSN means "something committed up to here", not "something you may read": poll
    /// the feed source for the principal and treat an empty answer as normal.
    pub fn wait_after(&self, seen: u64, timeout: Duration) -> Result<u64, WaitError> {
        let mut left = timeout;
        let mut state = locked(&self.state);
        // The planted defect skips the look at the published value before the first sleep, which is
        // the lost wake: a commit that landed before the waiter arrived is waited for in vain.
        let mut looked = !bugs::active(Bug::NotifyWaitIgnoresPublished);
        let mut counted = false;
        let result = loop {
            if state.closed {
                break Err(WaitError::Closed);
            }
            if looked && state.latest > seen {
                break Ok(state.latest);
            }
            looked = true;
            if left.is_zero() {
                break Err(WaitError::TimedOut);
            }
            if !counted {
                if state.sleepers >= self.max_waiters {
                    break Err(WaitError::Full);
                }
                state.sleepers += 1;
                counted = true;
            }
            let (guard, timed) = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner);
            state = guard;
            if timed.timed_out() {
                left = Duration::ZERO;
            }
        };
        if counted {
            state.sleepers -= 1;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    fn spawn<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> thread::JoinHandle<T> {
        thread::Builder::new().spawn(f).unwrap()
    }

    /// Waits without a sleep call: a receive on a channel nobody sends to.
    fn pause(ms: u64) {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let _ = rx.recv_timeout(Duration::from_millis(ms));
        drop(tx);
    }

    const LONG: Duration = Duration::from_secs(120);

    fn until(mut condition: impl FnMut() -> bool) {
        for _ in 0..20_000 {
            if condition() {
                return;
            }
            pause(1);
        }
        panic!("condition not reached");
    }

    #[test]
    fn a_waiter_registering_after_the_commit_returns_at_once() {
        let n = CommitNotifier::new(4);
        n.publish(7);
        // A zero timeout cannot sleep, so only the look at the published value can answer.
        assert_eq!(n.wait_after(0, Duration::ZERO), Ok(7));
        assert_eq!(n.wait_after(6, Duration::ZERO), Ok(7));
        assert_eq!(n.sleepers(), 0);
    }

    #[test]
    fn a_waiter_that_has_seen_the_newest_lsn_sleeps_until_the_timeout() {
        let n = CommitNotifier::new(4);
        n.publish(7);
        assert_eq!(n.wait_after(7, Duration::ZERO), Err(WaitError::TimedOut));
        assert_eq!(n.wait_after(9, Duration::ZERO), Err(WaitError::TimedOut));
        assert_eq!(
            n.wait_after(7, Duration::from_millis(30)),
            Err(WaitError::TimedOut)
        );
        assert_eq!(n.sleepers(), 0);
    }

    #[test]
    fn the_published_lsn_never_goes_back() {
        let n = CommitNotifier::new(4);
        n.publish(5);
        n.publish(3);
        n.publish(5);
        assert_eq!(n.latest(), 5);
        n.publish(6);
        assert_eq!(n.latest(), 6);
    }

    #[test]
    fn a_commit_wakes_a_sleeper_with_the_newest_lsn() {
        let n = Arc::new(CommitNotifier::new(4));
        let waiter = {
            let n = n.clone();
            spawn(move || n.wait_after(0, LONG))
        };
        until(|| n.sleepers() == 1);
        n.publish(2);
        n.publish(3);
        let got = waiter.join().unwrap();
        // It woke on 2 or 3 depending on timing, never on a stale value and never on nothing.
        assert!(matches!(got, Ok(2 | 3)), "{got:?}");
        assert_eq!(n.sleepers(), 0);
    }

    #[test]
    fn each_waiter_wakes_only_for_an_lsn_past_its_own() {
        let n = Arc::new(CommitNotifier::new(8));
        let woke = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for seen in [0_u64, 10, 20] {
            let (n, woke) = (n.clone(), woke.clone());
            handles.push(spawn(move || {
                let got = n.wait_after(seen, LONG);
                woke.fetch_add(1, Ordering::SeqCst);
                got
            }));
        }
        until(|| n.sleepers() == 3);
        n.publish(5);
        until(|| woke.load(Ordering::SeqCst) == 1);
        // Neither of the others may wake for 5; give a wrong wake time to show itself.
        pause(50);
        assert_eq!(woke.load(Ordering::SeqCst), 1);
        n.publish(15);
        until(|| woke.load(Ordering::SeqCst) == 2);
        n.publish(25);
        let got: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(got, vec![Ok(5), Ok(15), Ok(25)]);
    }

    #[test]
    fn the_sleeper_cap_refuses_the_next_waiter_without_blocking_it() {
        let n = Arc::new(CommitNotifier::new(2));
        let held: Vec<_> = (0..2)
            .map(|_| {
                let n = n.clone();
                spawn(move || n.wait_after(0, LONG))
            })
            .collect();
        until(|| n.sleepers() == 2);
        assert_eq!(n.wait_after(0, LONG), Err(WaitError::Full));
        n.publish(1);
        for h in held {
            assert_eq!(h.join().unwrap(), Ok(1));
        }
        // A waiter that does not need to sleep is never counted against the cap.
        assert_eq!(n.wait_after(0, LONG), Ok(1));
        assert_eq!(n.wait_after(1, Duration::ZERO), Err(WaitError::TimedOut));
        // A refused or finished waiter leaves no count behind.
        assert_eq!(n.sleepers(), 0);
        assert_eq!(CommitNotifier::new(0).max_waiters, 1);
    }

    #[test]
    fn closing_wakes_every_sleeper_and_refuses_later_waits() {
        let n = Arc::new(CommitNotifier::new(8));
        let handles: Vec<_> = (0..5)
            .map(|_| {
                let n = n.clone();
                spawn(move || n.wait_after(0, LONG))
            })
            .collect();
        until(|| n.sleepers() == 5);
        n.close();
        for h in handles {
            assert_eq!(h.join().unwrap(), Err(WaitError::Closed));
        }
        assert_eq!(n.sleepers(), 0);
        // Closed wins over a pending commit and over a zero timeout.
        n.publish(9);
        assert_eq!(n.wait_after(0, LONG), Err(WaitError::Closed));
        assert_eq!(n.wait_after(0, Duration::ZERO), Err(WaitError::Closed));
    }

    #[test]
    fn a_spurious_wake_is_absorbed() {
        let n = Arc::new(CommitNotifier::new(4));
        let waiter = {
            let n = n.clone();
            spawn(move || n.wait_after(5, LONG))
        };
        until(|| n.sleepers() == 1);
        // Wake it with nothing newer than what it has seen: it must go back to sleep.
        n.changed.notify_all();
        n.publish(5);
        pause(30);
        assert_eq!(n.sleepers(), 1);
        n.publish(6);
        assert_eq!(waiter.join().unwrap(), Ok(6));
    }

    /// Model: a wait's answer is a function of (published, seen, closed) alone. Drive a
    /// pseudo-random sequence of single-threaded operations and compare every wait with the model.
    #[test]
    fn waits_match_a_model_of_published_seen_and_closed() {
        let n = CommitNotifier::new(4);
        let (mut model_latest, mut model_closed) = (0_u64, false);
        let mut x = 0x2545_f491_4f6c_dd1d_u64;
        for step in 0..2000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let value = x % 40;
            match x % 7 {
                0 | 1 => {
                    n.publish(value);
                    model_latest = model_latest.max(value);
                }
                2 if step % 500 == 0 => {
                    n.close();
                    model_closed = true;
                }
                _ => {
                    let expected = if model_closed {
                        Err(WaitError::Closed)
                    } else if model_latest > value {
                        Ok(model_latest)
                    } else {
                        Err(WaitError::TimedOut)
                    };
                    assert_eq!(n.wait_after(value, Duration::ZERO), expected);
                }
            }
            assert_eq!(n.latest(), model_latest);
        }
    }

    /// Many publishers and waiters, each waiter chasing the newest LSN to the final one. A lost wake
    /// would leave a waiter asleep until its timeout, so the (long) timeout is the failure signal:
    /// a waiter that needs it has missed a commit.
    #[test]
    fn no_wake_is_lost_under_a_stress_interleaving() {
        const PUBLISHERS: u64 = 4;
        const PER: u64 = 400;
        let n = Arc::new(CommitNotifier::new(64));
        let top = Arc::new(Mutex::new(0_u64));
        let mut handles = Vec::new();
        for _ in 0..24 {
            let n = n.clone();
            handles.push(spawn(move || {
                let mut seen = 0;
                let mut wakes = 0_u64;
                while seen < PUBLISHERS * PER {
                    match n.wait_after(seen, Duration::from_secs(30)) {
                        Ok(lsn) => {
                            assert!(lsn > seen);
                            seen = lsn;
                            wakes += 1;
                        }
                        Err(e) => panic!("missed a commit: {e:?} after {seen}"),
                    }
                }
                wakes
            }));
        }
        let mut publishers = Vec::new();
        for p in 0..PUBLISHERS {
            let (n, top) = (n.clone(), top.clone());
            publishers.push(spawn(move || {
                for i in 0..PER {
                    // The next LSN is taken and published under one lock, so publishes arrive in
                    // order as they do from the single actor.
                    let mut next = top.lock().unwrap();
                    *next += 1;
                    n.publish(*next);
                    drop(next);
                    if (i + p) % 7 == 0 {
                        thread::yield_now();
                    }
                }
            }));
        }
        for p in publishers {
            p.join().unwrap();
        }
        for h in handles {
            assert!(h.join().unwrap() >= 1);
        }
        assert_eq!(n.latest(), PUBLISHERS * PER);
        assert_eq!(n.sleepers(), 0);
    }

    #[test]
    fn a_planted_skipped_look_loses_the_wake_of_an_earlier_commit() {
        bugs::clear();
        bugs::enable(Bug::NotifyWaitIgnoresPublished);
        let n = CommitNotifier::new(4);
        n.publish(7);
        // The commit is already there, yet the defective waiter does not see it.
        assert_eq!(n.wait_after(0, Duration::ZERO), Err(WaitError::TimedOut));
        bugs::clear();
        assert_eq!(n.wait_after(0, Duration::ZERO), Ok(7));
    }
}
