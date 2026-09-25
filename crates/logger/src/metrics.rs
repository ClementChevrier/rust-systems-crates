//! Process-wide counters for observing the logger while it runs.
//!
//! Counters cover one window — the span between two footers — and are reset
//! when that footer is written. A rotated log file therefore carries its own
//! tally and reads on its own, with no subtraction against the previous file.
//! The two gauges, `current_file_bytes` and `consecutive_write_failures`,
//! describe the present and are never reset.
//!
//! Relaxed ordering throughout: these carry no data and order nothing. A
//! snapshot is therefore not a consistent instant, two fields may come from
//! adjacent moments. That is the right trade for observability on a hot path.

use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! counters {
    ($($name:ident),+ $(,)?) => {
        $( static $name: AtomicU64 = AtomicU64::new(0); )+
    };
}

counters!(
    ENQUEUED,
    DROPPED,
    BYTES_DROPPED,
    WRITTEN,
    BYTES_WRITTEN,
    BYTES_LOST,
    WRITE_FAILURES,
    FSYNC_FAILURES,
    ROTATION_FAILURES,
    CURRENT_FILE_BYTES,
    CONSECUTIVE_WRITE_FAILURES,
);

/// Drops not yet reported in the log itself. Separate from `DROPPED`, which
/// belongs to the footer window and is reset on rotation.
static UNREPORTED_DROPS: AtomicU64 = AtomicU64::new(0);

/// Takes the drops since the last call, for the worker to write them down.
/// A plain load first: in steady state there is nothing to take, and no
/// read-modify-write is paid.
pub(crate) fn take_unreported_drops() -> u64 {
    if UNREPORTED_DROPS.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    UNREPORTED_DROPS.swap(0, Ordering::Relaxed)
}

fn bump(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::Relaxed) + 1
}

pub(crate) fn record_enqueued() {
    bump(&ENQUEUED);
}
pub(crate) fn record_rotation_failure() {
    bump(&ROTATION_FAILURES);
}

/// Returns the new total so the caller can rate-limit its own reporting.
pub(crate) fn record_write_failure() -> u64 {
    bump(&WRITE_FAILURES)
}
pub(crate) fn record_fsync_failure() -> u64 {
    bump(&FSYNC_FAILURES)
}

pub(crate) fn record_drop(encoded_len: usize) {
    DROPPED.fetch_add(1, Ordering::Relaxed);
    UNREPORTED_DROPS.fetch_add(1, Ordering::Relaxed);
    BYTES_DROPPED.fetch_add(encoded_len as u64, Ordering::Relaxed);
}

pub(crate) fn record_written(bytes: u64, lost: u64) {
    WRITTEN.fetch_add(1, Ordering::Relaxed);
    BYTES_WRITTEN.fetch_add(bytes, Ordering::Relaxed);
    if lost != 0 {
        BYTES_LOST.fetch_add(lost, Ordering::Relaxed);
    }
}

pub(crate) fn set_file_bytes(bytes: u64) {
    CURRENT_FILE_BYTES.store(bytes, Ordering::Relaxed);
}

pub(crate) fn set_write_streak(count: u64) {
    CONSECUTIVE_WRITE_FAILURES.store(count, Ordering::Relaxed);
}

/// Point-in-time view of the logger's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoggerMetrics {
    /// Messages accepted into the channel.
    pub enqueued: u64,
    /// Messages refused because the channel was full.
    pub dropped: u64,
    /// Encoded size of the refused messages.
    pub bytes_dropped: u64,

    /// Writes issued to the file: one per buffer flush, not one per message.
    pub written: u64,
    /// Bytes the file accepted.
    pub bytes_written: u64,
    /// Bytes the file refused on a partial or failed write.
    pub bytes_lost: u64,

    /// Writes that failed, entirely or partially.
    pub write_failures: u64,
    /// `fsync` calls that failed.
    pub fsync_failures: u64,
    /// Rotations that could not open the next file; the logger keeps writing
    /// to the current one.
    pub rotation_failures: u64,

    /// Size of the file currently being written.
    pub current_file_bytes: u64,
    /// Failed writes since the last complete one. Non-zero means the logger is
    /// losing lines *right now*.
    pub consecutive_write_failures: u64,
}

impl LoggerMetrics {
    /// Nothing has been lost during this window (since the last footer).
    pub fn is_lossless(&self) -> bool {
        self.dropped == 0 && self.bytes_lost == 0
    }

    /// The logger is failing to write at this instant.
    pub fn is_degraded(&self) -> bool {
        self.consecutive_write_failures != 0 || self.rotation_failures != 0
    }
}

impl std::fmt::Display for LoggerMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "in={} writes={} bytes={} file={}",
            self.enqueued, self.written, self.bytes_written, self.current_file_bytes,
        )?;

        if !self.is_lossless() {
            write!(
                f,
                " | LOST dropped={} dropped_bytes={} refused_bytes={}",
                self.dropped, self.bytes_dropped, self.bytes_lost,
            )?;
        }

        if self.is_degraded() {
            write!(
                f,
                " | DEGRADED write_streak={} write_failures={} fsync_failures={} rotation_failures={}",
                self.consecutive_write_failures,
                self.write_failures,
                self.fsync_failures,
                self.rotation_failures,
            )?;
        }

        Ok(())
    }
}

/// Reads every counter without resetting it. See [`crate::Logger::peek_metrics`].
pub fn peek() -> LoggerMetrics {
    let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
    LoggerMetrics {
        enqueued: load(&ENQUEUED),
        dropped: load(&DROPPED),
        bytes_dropped: load(&BYTES_DROPPED),
        written: load(&WRITTEN),
        bytes_written: load(&BYTES_WRITTEN),
        bytes_lost: load(&BYTES_LOST),
        write_failures: load(&WRITE_FAILURES),
        fsync_failures: load(&FSYNC_FAILURES),
        rotation_failures: load(&ROTATION_FAILURES),
        current_file_bytes: load(&CURRENT_FILE_BYTES),
        consecutive_write_failures: load(&CONSECUTIVE_WRITE_FAILURES),
    }
}

pub(crate) fn collect() -> LoggerMetrics {
    let swap = |counter: &AtomicU64| counter.swap(0, Ordering::Relaxed);
    LoggerMetrics {
        enqueued: swap(&ENQUEUED),
        dropped: swap(&DROPPED),
        bytes_dropped: swap(&BYTES_DROPPED),
        written: swap(&WRITTEN),
        bytes_written: swap(&BYTES_WRITTEN),
        bytes_lost: swap(&BYTES_LOST),
        write_failures: swap(&WRITE_FAILURES),
        fsync_failures: swap(&FSYNC_FAILURES),
        rotation_failures: swap(&ROTATION_FAILURES),
        // Gauges, not counters: they describe the present and are not reset.
        current_file_bytes: CURRENT_FILE_BYTES.load(Ordering::Relaxed),
        consecutive_write_failures: CONSECUTIVE_WRITE_FAILURES.load(Ordering::Relaxed),
    }
}
