//! Thread priority levels and the boost policy.
//!
//! Windows computes a thread's base priority from the process priority class
//! ([`ProcessClass`](crate::process::ProcessClass)) and the thread priority
//! level ([`ThreadPriority`]). Combining [`crate::process::set_class`] with
//! [`crate::thread::Builder::priority`] reaches any row of this table:
//!
//! | Process class | Thread level  | Base priority |
//! |---------------|---------------|---------------|
//! | IDLE          | IDLE          | 1             |
//! | IDLE          | LOWEST        | 2             |
//! | IDLE          | BELOW_NORMAL  | 3             |
//! | IDLE          | NORMAL        | 4             |
//! | IDLE          | ABOVE_NORMAL  | 5             |
//! | IDLE          | HIGHEST       | 6             |
//! | IDLE          | TIME_CRITICAL | 15            |
//! | BELOW_NORMAL  | IDLE          | 1             |
//! | BELOW_NORMAL  | LOWEST        | 4             |
//! | BELOW_NORMAL  | BELOW_NORMAL  | 5             |
//! | BELOW_NORMAL  | NORMAL        | 6             |
//! | BELOW_NORMAL  | ABOVE_NORMAL  | 7             |
//! | BELOW_NORMAL  | HIGHEST       | 8             |
//! | BELOW_NORMAL  | TIME_CRITICAL | 15            |
//! | NORMAL        | IDLE          | 1             |
//! | NORMAL        | LOWEST        | 6             |
//! | NORMAL        | BELOW_NORMAL  | 7             |
//! | NORMAL        | NORMAL        | 8             |
//! | NORMAL        | ABOVE_NORMAL  | 9             |
//! | NORMAL        | HIGHEST       | 10            |
//! | NORMAL        | TIME_CRITICAL | 15            |
//! | ABOVE_NORMAL  | IDLE          | 1             |
//! | ABOVE_NORMAL  | LOWEST        | 8             |
//! | ABOVE_NORMAL  | BELOW_NORMAL  | 9             |
//! | ABOVE_NORMAL  | NORMAL        | 10            |
//! | ABOVE_NORMAL  | ABOVE_NORMAL  | 11            |
//! | ABOVE_NORMAL  | HIGHEST       | 12            |
//! | ABOVE_NORMAL  | TIME_CRITICAL | 15            |
//! | HIGH          | IDLE          | 1             |
//! | HIGH          | LOWEST        | 11            |
//! | HIGH          | BELOW_NORMAL  | 12            |
//! | HIGH          | NORMAL        | 13            |
//! | HIGH          | ABOVE_NORMAL  | 14            |
//! | HIGH          | HIGHEST       | 15            |
//! | HIGH          | TIME_CRITICAL | 15            |
//! | REALTIME      | IDLE          | 16            |
//! | REALTIME      | LOWEST        | 22            |
//! | REALTIME      | BELOW_NORMAL  | 23            |
//! | REALTIME      | NORMAL        | 24            |
//! | REALTIME      | ABOVE_NORMAL  | 25            |
//! | REALTIME      | HIGHEST       | 26            |
//! | REALTIME      | TIME_CRITICAL | 31            |
//!
//! See <https://learn.microsoft.com/en-us/windows/win32/procthread/scheduling-priorities>.

/// Win32 thread priority levels, mapping 1:1 onto `THREAD_PRIORITY_*`.
///
/// A level is not an absolute priority: it combines with the process class to
/// produce the base priority, per the table at the top of this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadPriority {
    /// `THREAD_PRIORITY_IDLE`: base priority 1, or 16 in a real-time process.
    Idle,
    /// `THREAD_PRIORITY_LOWEST`: two below the class's normal level.
    Lowest,
    /// `THREAD_PRIORITY_BELOW_NORMAL`: one below the class's normal level.
    BelowNormal,
    /// `THREAD_PRIORITY_NORMAL`: the class's normal level.
    Normal,
    /// `THREAD_PRIORITY_ABOVE_NORMAL`: one above the class's normal level.
    AboveNormal,
    /// `THREAD_PRIORITY_HIGHEST`: two above the class's normal level.
    Highest,
    /// `THREAD_PRIORITY_TIME_CRITICAL`: base priority 15, or 31 in a real-time
    /// process.
    TimeCritical,
}

impl ThreadPriority {
    pub(crate) fn as_win32(self) -> i32 {
        match self {
            Self::Idle => -15,
            Self::Lowest => -2,
            Self::BelowNormal => -1,
            Self::Normal => 0,
            Self::AboveNormal => 1,
            Self::Highest => 2,
            Self::TimeCritical => 15,
        }
    }

    fn from_win32(value: i32) -> Option<Self> {
        match value {
            -15 => Some(Self::Idle),
            -2 => Some(Self::Lowest),
            -1 => Some(Self::BelowNormal),
            0 => Some(Self::Normal),
            1 => Some(Self::AboveNormal),
            2 => Some(Self::Highest),
            15 => Some(Self::TimeCritical),
            _ => None,
        }
    }
}

impl std::fmt::Display for ThreadPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Idle => "Idle",
            Self::Lowest => "Lowest",
            Self::BelowNormal => "BelowNormal",
            Self::Normal => "Normal",
            Self::AboveNormal => "AboveNormal",
            Self::Highest => "Highest",
            Self::TimeCritical => "TimeCritical",
        })
    }
}

/// A thread priority level exactly as `GetThreadPriority` returns it.
///
/// It is the level relative to the process class, not the base priority. The
/// seven standard levels map back to a [`ThreadPriority`]; a thread in a
/// real-time process can also sit at the intermediate levels -7 to -3 and 3 to
/// 6, which have no name, so the raw value is what gets stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawThreadPriority(i32);

impl RawThreadPriority {
    pub(crate) fn new(value: i32) -> Self {
        Self(value)
    }

    /// The value returned by `GetThreadPriority`.
    pub fn get(self) -> i32 {
        self.0
    }

    /// The named level, when the value is one of the seven standard ones.
    pub fn level(self) -> Option<ThreadPriority> {
        ThreadPriority::from_win32(self.0)
    }
}

impl std::fmt::Display for RawThreadPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.level() {
            Some(level) => write!(f, "{level}"),
            None => write!(f, "{}", self.0),
        }
    }
}

/// Windows dynamic priority boost.
///
/// Windows briefly raises the priority of a thread coming out of a wait. Useful
/// for interactive work, harmful for a thread whose latency must be
/// predictable: its priority then shifts underneath the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoostPolicy {
    /// Windows may boost the thread (the default).
    Enabled,
    /// The thread keeps its base priority.
    Disabled,
}

impl BoostPolicy {
    /// The `bDisablePriorityBoost` argument: `TRUE` disables the boost.
    pub(crate) fn as_win32(self) -> i32 {
        i32::from(self == Self::Disabled)
    }

    /// From the `bDisablePriorityBoost` value Windows reports.
    pub(crate) fn from_win32(disabled: i32) -> Self {
        if disabled == 0 {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

impl std::fmt::Display for BoostPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Enabled => "on",
            Self::Disabled => "off",
        })
    }
}
