#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;
use crate::thread::{Builder, priority::ThreadPriority};
use std::{
    os::windows::io::AsRawHandle,
    sync::mpsc::channel,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(5);

/// The registry purge relies on `alive`. If `is_alive` never flipped, dead
/// entries would never leave and their handles would leak.
#[test]
fn is_alive_flips_once_the_thread_has_exited() {
    let spawned = Builder::new().spawn(|| {}).expect("spawn");
    let raw = spawned.handle.as_raw_handle();

    let deadline = Instant::now() + DEADLINE;
    while is_alive(raw).expect("is_alive must not fail on a valid handle") {
        assert!(
            Instant::now() < deadline,
            "an exited thread must eventually be reported"
        );
        std::thread::yield_now();
    }
    spawned.handle.join().expect("thread must not panic");
}

/// An invalid handle must produce an error, never a panic in the monitoring
/// thread.
#[test]
fn is_alive_on_an_invalid_handle_reports_an_error() {
    assert!(is_alive(std::ptr::null_mut()).is_err());
}

/// `GetThreadPriority` returns the level, whatever the process class, so the
/// raw value must round-trip to the level that was set. Checked on a dedicated
/// thread, without touching the process class other tests depend on.
#[test]
fn priority_reads_back_the_level_that_was_set() {
    for level in [
        ThreadPriority::Idle,
        ThreadPriority::BelowNormal,
        ThreadPriority::Highest,
        ThreadPriority::TimeCritical,
    ] {
        let (tx, rx) = channel();
        let spawned = Builder::new()
            .priority(level)
            .spawn(move || {
                // SAFETY: the current thread's pseudo-handle is always valid.
                let _ = tx.send(priority(unsafe { GetCurrentThread() }).expect("priority"));
            })
            .expect("spawn");

        let raw = rx.recv_timeout(DEADLINE).expect("thread must report");
        assert_eq!(raw.get(), level.as_win32(), "{level}");
        assert_eq!(raw.level(), Some(level));
        spawned.handle.join().expect("thread must not panic");
    }
}
