//! Thread CPU time, from `GetThreadTimes`.

use std::{
    fmt::Write,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::fmt::duration;

/// `FILETIME`: a count of 100 ns ticks, split into two 32-bit words.
#[repr(C)]
#[derive(Default)]
pub(crate) struct C_FileTime {
    dw_low_date_time: u32,
    dw_high_date_time: u32,
}
impl C_FileTime {
    pub(crate) fn into_u64(self) -> u64 {
        ((self.dw_high_date_time as u64) << 32) | self.dw_low_date_time as u64
    }

    /// As an absolute time. A value before 1970 saturates to the Unix epoch
    /// rather than panicking.
    pub(crate) fn into_system_time(self) -> SystemTime {
        // 100 ns ticks between 1601-01-01 and 1970-01-01.
        const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
        let t = self.into_u64().saturating_sub(EPOCH_DIFF);
        UNIX_EPOCH + Duration::from_nanos_u128(t as u128 * 100u128)
    }

    /// As a span of time.
    pub(crate) fn into_duration(self) -> Duration {
        let t = self.into_u64() as u128;
        Duration::from_nanos_u128(t * 100u128)
    }
}

/// A thread's creation time and consumed CPU time, read through `GetThreadTimes`.
#[derive(Copy, Clone, Debug)]
pub struct ThreadTimes {
    /// When the thread was created.
    pub creation: SystemTime,
    /// CPU time spent in kernel mode.
    pub kernel: Duration,
    /// CPU time spent in user mode.
    pub user: Duration,
}
impl ThreadTimes {
    /// Total CPU time consumed: kernel plus user.
    ///
    /// Not to be confused with the thread's age: a blocked thread ages without
    /// consuming any CPU.
    pub fn cpu(&self) -> Duration {
        self.kernel + self.user
    }

    /// A fixed-width console report of these times.
    pub fn ascii_report(&self) -> String {
        let mut out = String::new();
        let age = self.creation.elapsed().unwrap_or_default();

        // Writing into a String cannot fail, so the results are ignored.
        let _ = writeln!(out, "╔════════════════════════════════════════════════╗");
        let _ = writeln!(out, "║                  THREAD TIME                   ║");
        let _ = writeln!(out, "╠════════════════════════════════════════════════╣");
        let _ = writeln!(out, "║ CPU                                            ║");
        let _ = writeln!(
            out,
            "║   Kernel ·························{:>12} ║",
            duration(self.kernel)
        );
        let _ = writeln!(
            out,
            "║   User ···························{:>12} ║",
            duration(self.user)
        );
        let _ = writeln!(
            out,
            "║   Total ··························{:>12} ║",
            duration(self.cpu())
        );
        let _ = writeln!(out, "║                                                ║");
        let _ = writeln!(out, "║ LIFETIME                                       ║");
        let _ = writeln!(
            out,
            "║   Age ····························{:>12} ║",
            duration(age)
        );
        let _ = write!(out, "╚════════════════════════════════════════════════╝");
        out
    }
}

#[cfg(test)]
#[path = "tests/times.rs"]
mod times_test;
