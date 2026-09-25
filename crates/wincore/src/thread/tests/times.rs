#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

const TICKS_PER_SEC: u64 = 10_000_000;

fn file_time(ticks: u64) -> C_FileTime {
    C_FileTime {
        dw_low_date_time: ticks as u32,
        dw_high_date_time: (ticks >> 32) as u32,
    }
}

/// Exercises the high/low recomposition: a value that fits in the low word
/// proves nothing, since `dw_high_date_time` could be ignored unnoticed.
#[test]
fn filetime_recomposes_both_words() {
    assert_eq!(
        file_time(TICKS_PER_SEC).into_duration(),
        Duration::from_secs(1)
    );

    let ticks = (1u64 << 32) + 5;
    assert_eq!(
        file_time(ticks).into_duration(),
        Duration::new(429, 496_730_100)
    );
}

/// Regression: a time before 1970 used to hit an `expect` and panic. A VM with
/// a skewed clock was enough to crash a simple diagnostic call.
#[test]
fn filetime_before_the_unix_epoch_does_not_panic() {
    let _ = file_time(0).into_system_time();
}
