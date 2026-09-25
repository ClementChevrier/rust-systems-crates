//! Human-readable formatting shared by the console reports.

use std::time::Duration;

const SECS_PER_HOUR: u64 = 3_600;
const SECS_PER_MIN: u64 = 60;
const MILLIS_PER_SEC: f64 = 1_000.0;

const KIB: f64 = 1024.0;
const MIB: f64 = 1024.0 * KIB;
const GIB: f64 = 1024.0 * MIB;

/// `1h02m03s`, `2m03s`, `3.042s` or `0.125ms`, whichever fits.
pub(crate) fn duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= SECS_PER_HOUR {
        format!(
            "{}h{:02}m{:02}s",
            secs / SECS_PER_HOUR,
            (secs % SECS_PER_HOUR) / SECS_PER_MIN,
            secs % SECS_PER_MIN
        )
    } else if secs >= SECS_PER_MIN {
        format!("{}m{:02}s", secs / SECS_PER_MIN, secs % SECS_PER_MIN)
    } else if secs > 0 {
        format!("{secs}.{:03}s", d.subsec_millis())
    } else {
        format!("{:.3}ms", d.as_secs_f64() * MILLIS_PER_SEC)
    }
}

/// A byte count in B, KB, MB or GB (powers of 1024), whichever fits.
pub(crate) fn bytes(value: usize) -> String {
    let value = value as f64;
    if value >= GIB {
        format!("{:.2} GB", value / GIB)
    } else if value >= MIB {
        format!("{:.2} MB", value / MIB)
    } else if value >= KIB {
        format!("{:.2} KB", value / KIB)
    } else {
        format!("{value:.0} B")
    }
}

/// A cycle count with a K, M or G suffix (powers of 1000).
pub(crate) fn cycles(value: u64) -> String {
    const KILO: f64 = 1_000.0;
    const MEGA: f64 = 1_000.0 * KILO;
    const GIGA: f64 = 1_000.0 * MEGA;

    let count = value as f64;
    if count >= GIGA {
        format!("{:.2}G", count / GIGA)
    } else if count >= MEGA {
        format!("{:.2}M", count / MEGA)
    } else if count >= KILO {
        format!("{:.2}K", count / KILO)
    } else {
        format!("{value}")
    }
}
