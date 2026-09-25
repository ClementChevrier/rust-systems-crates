#![allow(clippy::unwrap_used, clippy::panic)]

//! Windows tests: they check the real effect of the configuration through kernel32.

use super::*;
use std::sync::mpsc::channel;
use std::time::Duration;

const RECV_TIMEOUT: Duration = Duration::from_secs(5);
/// Past the width of KAFFINITY: no CPU can have this number.
const OUT_OF_RANGE_CPU: u8 = usize::BITS as u8;

mod ffi {
    use std::os::raw::c_void;

    // Signatures as documented for kernel32.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub(super) fn GetCurrentThread() -> *mut c_void;
        pub(super) fn GetThreadPriority(hThread: *mut c_void) -> i32;
        pub(super) fn GetCurrentProcessorNumber() -> u32;
    }
}

fn observed_priority() -> i32 {
    // SAFETY: the current thread's pseudo-handle is always valid.
    unsafe { ffi::GetThreadPriority(ffi::GetCurrentThread()) }
}

fn observed_cpu() -> u32 {
    // SAFETY: no precondition.
    unsafe { ffi::GetCurrentProcessorNumber() }
}

/// The raw Windows value, bypassing `BoostPolicy`: TRUE means the boost is disabled.
fn observed_boost_disabled_raw() -> u32 {
    let mut disabled = 0u32;
    // SAFETY: valid pseudo-handle; `disabled` points at a live local.
    unsafe {
        crate::ffi::GetThreadPriorityBoost(
            ffi::GetCurrentThread(),
            &mut disabled as *mut _ as *mut i32,
        )
    };
    disabled
}

/// The contract `spawn` announces: the configuration is applied *before* `f`
/// runs. If it came after, the closure would read the default priority.
#[test]
fn config_is_applied_before_the_closure_runs() {
    let (tx, rx) = channel();

    let spawned = Builder::new()
        .priority(ThreadPriority::Highest)
        .spawn(move || {
            let _ = tx.send(observed_priority());
        })
        .expect("spawn must succeed");

    assert_eq!(
        rx.recv_timeout(RECV_TIMEOUT),
        Ok(ThreadPriority::Highest.as_win32())
    );
    spawned.handle.join().expect("thread must not panic");
}

#[test]
fn every_priority_is_observable_via_kernel32() {
    let priorities = [
        ThreadPriority::Idle,
        ThreadPriority::Lowest,
        ThreadPriority::BelowNormal,
        ThreadPriority::Normal,
        ThreadPriority::AboveNormal,
        ThreadPriority::Highest,
        ThreadPriority::TimeCritical,
    ];

    for prio in priorities {
        let (tx, rx) = channel();
        let spawned = Builder::new()
            .priority(prio)
            .spawn(move || {
                let _ = tx.send(observed_priority());
            })
            .expect("setting priority must succeed");

        assert_eq!(
            rx.recv_timeout(RECV_TIMEOUT),
            Ok(prio.as_win32()),
            "priority {prio}"
        );
        spawned.handle.join().expect("thread must not panic");
    }
}

/// Regression: the original `or_else` chain silently skipped the priority as
/// soon as the pin failed, and the worker ran with neither pin *nor* priority.
#[test]
fn a_failed_pin_does_not_skip_the_priority() {
    let (tx, rx) = channel();

    let spawned = Builder::new()
        .pin_to(ProcessorId {
            number: OUT_OF_RANGE_CPU,
            group: 0,
        })
        .priority(ThreadPriority::Highest)
        .spawn(move || {
            let _ = tx.send(observed_priority());
        })
        .expect("spawn must succeed even when pinning fails");

    assert_eq!(
        rx.recv_timeout(RECV_TIMEOUT),
        Ok(ThreadPriority::Highest.as_win32()),
        "a failed pin must not prevent the priority from being applied",
    );
    assert!(
        spawned.config.pin.failure().is_some(),
        "the pin must be reported as failed"
    );
    assert!(
        spawned.config.prio.failure().is_none(),
        "the priority must be reported as applied"
    );
    spawned.handle.join().expect("thread must not panic");
}

/// A log line must stand on its own: the requested CPU must survive in the
/// failure, otherwise it reads "pin failed" without saying which CPU was meant.
#[test]
fn a_failed_pin_reports_the_requested_cpu() {
    let wanted = ProcessorId {
        number: OUT_OF_RANGE_CPU,
        group: 0,
    };

    let spawned = Builder::new()
        .pin_to(wanted)
        .spawn(|| {})
        .expect("spawn must succeed even when pinning fails");

    let (kind, _) = spawned
        .config
        .failures()
        .next()
        .expect("a failure must be reported");
    assert!(matches!(kind, SettingKind::Pin));
    assert_eq!(spawned.config.pin.wanted(), Some(&wanted));
    assert!(
        spawned
            .config
            .to_string()
            .contains(&wanted.number.to_string())
    );
    spawned.handle.join().expect("thread must not panic");
}

/// Regression: the pin bound used to be `u8::BITS`, so on any machine with more
/// than eight logical CPUs, CPUs past the 7th were refused without asking Windows.
#[test]
fn pin_to_the_last_available_cpu_lands_there() {
    let cpus = thread::available_parallelism().expect("cpu count").get() as u32;
    let last = cpus - 1;
    let (tx, rx) = channel();

    let spawned = Builder::new()
        .pin_to(ProcessorId {
            number: last as u8,
            group: 0,
        })
        .spawn(move || {
            let _ = tx.send(observed_cpu());
        })
        .expect("spawn must succeed");

    assert!(
        spawned.config.pin.failure().is_none(),
        "pin refused on CPU {last}"
    );
    assert_eq!(rx.recv_timeout(RECV_TIMEOUT), Ok(last));
    spawned.handle.join().expect("thread must not panic");
}

/// Regression: the conversions to and from Windows were once both inverted, so a
/// `BoostPolicy -> BoostPolicy` round trip would still pass. This test compares
/// with the raw Windows value instead, where TRUE means "boost disabled".
#[test]
fn disabling_boost_actually_disables_it_in_windows() {
    let (tx, rx) = channel();

    let spawned = Builder::new()
        .boost_policy(BoostPolicy::Disabled)
        .spawn(move || {
            let _ = tx.send(observed_boost_disabled_raw());
        })
        .expect("spawn must succeed");

    assert_ne!(
        rx.recv_timeout(RECV_TIMEOUT).expect("thread must report"),
        0,
        "BoostPolicy::Disabled must translate to bDisablePriorityBoost = TRUE",
    );
    spawned.handle.join().expect("thread must not panic");
}

/// The only test exercising the `sync_channel` handshake under concurrency:
/// 32 spawns, each blocking on its own `recv`.
#[test]
fn a_burst_of_pinned_spawns_all_land_on_their_cpu() {
    const SPAWNS: u32 = 32;
    let cpus = thread::available_parallelism().expect("cpu count").get() as u32;
    let (tx, rx) = channel();

    let spawns: Vec<_> = (0..SPAWNS)
        .map(|i| {
            let assigned = i % cpus;
            let tx = tx.clone();
            Builder::new()
                .pin_to(ProcessorId {
                    number: assigned as u8,
                    group: 0,
                })
                .spawn(move || {
                    let _ = tx.send((assigned, observed_cpu()));
                })
                .expect("burst spawn must succeed")
        })
        .collect();
    drop(tx);

    let mut seen = 0;
    while let Ok((assigned, observed)) = rx.recv_timeout(RECV_TIMEOUT) {
        assert_eq!(observed, assigned);
        seen += 1;
    }
    assert_eq!(seen, SPAWNS);
    for spawned in spawns {
        spawned.handle.join().expect("thread must not panic");
    }
}
