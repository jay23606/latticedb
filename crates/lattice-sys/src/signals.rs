//! Stop-signal counting for the server process (SIGTERM and SIGINT).
//!
//! A service manager, a container runtime and a terminal all ask a process to stop with one of these
//! two signals. The handler does the only thing that is safe in a signal context: it increments an
//! atomic counter. The host polls the counter from a normal thread, so no allocation, lock or I/O
//! ever runs inside the handler. The counter is process-wide; the second signal is how an operator
//! asks for a forced exit while a graceful stop is still running.
//!
//! The libc symbol is declared by hand instead of through the `libc` crate. `std` already links the C
//! library on every unix target, so this adds no dependency and no lockfile entry. `signal(2)` is used
//! rather than `sigaction(2)` because its signature is identical on Linux (glibc and musl) and macOS,
//! while the `sigaction` struct layout is not; on those targets it installs a persistent handler.
//! The module is built only for Linux and macOS: System V `signal()` semantics (illumos) reset the
//! handler after the first delivery, which would break the second-signal rule.
//!
//! SIGHUP is a separate counter for a reload request (the bearer-token file, ADR 0031). It is installed
//! only when a caller asks for it, so a process that never asks keeps the default action of SIGHUP.

use std::io;
use std::sync::atomic::{AtomicU32, Ordering};

const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;
/// `SIG_ERR`, the all-ones value `signal(2)` returns on failure.
const SIG_ERR: usize = usize::MAX;

static STOP_SIGNALS: AtomicU32 = AtomicU32::new(0);
static RELOAD_SIGNALS: AtomicU32 = AtomicU32::new(0);

unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

extern "C" fn on_stop_signal(_signum: i32) {
    // A lock-free atomic add is async-signal-safe. Wrap-around is irrelevant: only 0, 1 and 2+ matter.
    STOP_SIGNALS.fetch_add(1, Ordering::SeqCst);
}

/// Installs counting handlers for SIGTERM and SIGINT. Calling it again is harmless.
///
/// Until it is called the default disposition applies (the process is killed by the signal).
pub fn install_stop_handlers() -> io::Result<()> {
    let handler = on_stop_signal as extern "C" fn(i32) as usize;
    for signum in [SIGTERM, SIGINT] {
        // SAFETY: `signal` is the C library function with this exact signature on every unix target.
        // `handler` is the address of an `extern "C"` function that only performs an atomic add, which
        // is async-signal-safe, and it lives for the whole program.
        let previous = unsafe { signal(signum, handler) };
        if previous == SIG_ERR {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// How many stop signals (SIGTERM or SIGINT) have arrived since the handlers were installed.
#[must_use]
pub fn stop_signals_received() -> u32 {
    STOP_SIGNALS.load(Ordering::SeqCst)
}

extern "C" fn on_reload_signal(_signum: i32) {
    RELOAD_SIGNALS.fetch_add(1, Ordering::SeqCst);
}

/// Installs a counting handler for SIGHUP, the reload request. Calling it again is harmless.
pub fn install_reload_handler() -> io::Result<()> {
    let handler = on_reload_signal as extern "C" fn(i32) as usize;
    // SAFETY: the same contract as `install_stop_handlers`: `signal` with this signature, and a handler
    // that only performs an atomic add and lives for the whole program.
    let previous = unsafe { signal(SIGHUP, handler) };
    if previous == SIG_ERR {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// How many SIGHUP reload requests have arrived since [`install_reload_handler`].
#[must_use]
pub fn reload_signals_received() -> u32 {
    RELOAD_SIGNALS.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    unsafe extern "C" {
        fn raise(signum: i32) -> i32;
    }

    #[test]
    fn each_signal_is_counted_once() {
        install_stop_handlers().unwrap();
        install_stop_handlers().unwrap();
        let before = stop_signals_received();
        for (n, signum) in [SIGTERM, SIGINT, SIGTERM].into_iter().enumerate() {
            // SAFETY: `raise` takes a signal number and runs the installed handler on this thread
            // before returning, so the count is final when the call returns.
            assert_eq!(unsafe { raise(signum) }, 0);
            assert_eq!(stop_signals_received(), before + n as u32 + 1);
        }
    }

    #[test]
    fn a_reload_request_is_counted_apart_from_stop_signals() {
        install_reload_handler().unwrap();
        let stops = stop_signals_received();
        let before = reload_signals_received();
        // SAFETY: as in `each_signal_is_counted_once`: the handler has run when `raise` returns.
        assert_eq!(unsafe { raise(SIGHUP) }, 0);
        assert_eq!(reload_signals_received(), before + 1);
        assert_eq!(stop_signals_received(), stops);
    }
}
