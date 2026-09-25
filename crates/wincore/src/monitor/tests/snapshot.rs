#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;
use crate::monitor::watched_count;

#[test]
fn dead_threads_leave_the_registry() {
    const ROUNDS: usize = 8;

    let before = watched_count();
    for _ in 0..ROUNDS {
        let spawned = crate::thread::Builder::new().spawn(|| {}).expect("spawn");
        spawned.handle.join().expect("thread must not panic");
    }
    let _ = Snapshot::capture();

    assert_eq!(
        watched_count(),
        before,
        "{ROUNDS} dead threads stayed in the registry"
    );
}

#[test]
fn the_calling_thread_can_watch_itself() {
    watch_current(ThreadConfig::default()).expect("enrol");

    let snapshot = Snapshot::capture();
    // SAFETY: no precondition.
    let me = unsafe { crate::ffi::GetCurrentThreadId() };
    assert!(snapshot.threads.iter().any(|report| report.tid == me));
}

#[test]
fn every_dashboard_line_has_the_same_width() {
    watch_current(ThreadConfig::default()).unwrap();
    let rendered = Snapshot::capture().ascii_report();
    let widths: Vec<usize> = rendered.lines().map(|line| line.chars().count()).collect();

    println!("{rendered}");
    assert!(
        widths.windows(2).all(|pair| pair[0] == pair[1]),
        "widths: {widths:?}"
    );
}
