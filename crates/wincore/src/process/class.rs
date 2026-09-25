use std::io;

use crate::ffi::{GetCurrentProcess, GetPriorityClass, HANDLE, SetPriorityClass};

/// Win32 process priority classes.
///
/// The class sets the band within which thread priority levels operate; see
/// the table in [`crate::thread::priority`]. `RealTime` places the process
/// above most drivers: use it knowingly, as a thread spinning there without
/// yielding can freeze the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProcessClass {
    /// `IDLE_PRIORITY_CLASS`: runs only when the system is idle.
    Idle,
    /// `BELOW_NORMAL_PRIORITY_CLASS`.
    BelowNormal,
    /// `NORMAL_PRIORITY_CLASS`: the default.
    Normal,
    /// `ABOVE_NORMAL_PRIORITY_CLASS`.
    AboveNormal,
    /// `HIGH_PRIORITY_CLASS`: preempts every normal process.
    High,
    /// `REALTIME_PRIORITY_CLASS`: above most system threads. Needs the
    /// "increase scheduling priority" privilege; without it, Windows silently
    /// grants `High` instead.
    RealTime,
}

impl ProcessClass {
    pub(crate) fn as_win32(self) -> u32 {
        match self {
            Self::Idle => 0x0000_0040,
            Self::BelowNormal => 0x0000_4000,
            Self::Normal => 0x0000_0020,
            Self::AboveNormal => 0x0000_8000,
            Self::High => 0x0000_0080,
            Self::RealTime => 0x0000_0100,
        }
    }

    pub(crate) fn from_win32(value: u32) -> Option<Self> {
        match value {
            0x0000_0040 => Some(Self::Idle),
            0x0000_4000 => Some(Self::BelowNormal),
            0x0000_0020 => Some(Self::Normal),
            0x0000_8000 => Some(Self::AboveNormal),
            0x0000_0080 => Some(Self::High),
            0x0000_0100 => Some(Self::RealTime),
            _ => None,
        }
    }
}

impl std::fmt::Display for ProcessClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Idle => "Idle",
            Self::BelowNormal => "BelowNormal",
            Self::Normal => "Normal",
            Self::AboveNormal => "AboveNormal",
            Self::High => "High",
            Self::RealTime => "RealTime",
        })
    }
}

/// Sets the priority class of the current process.
///
/// Reads the class back afterwards: Windows downgrades `RealTime` to `High`
/// without an error when the privilege is missing, and that case is reported
/// here as [`io::ErrorKind::PermissionDenied`].
pub fn set_class(level: ProcessClass) -> Result<(), io::Error> {
    // SAFETY: no precondition; returns a pseudo-handle that needs no closing.
    let h_process = unsafe { GetCurrentProcess() };
    // SAFETY: a valid process pseudo-handle and a documented class value.
    if unsafe { SetPriorityClass(h_process, level.as_win32()) } == 0 {
        return Err(io::Error::last_os_error());
    }

    let granted = class()?;
    if granted != level {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "asked for the {level} priority class, Windows granted {granted}; see \
                 https://learn.microsoft.com/en-us/windows/win32/procthread/setpriorityclass"
            ),
        ));
    }
    Ok(())
}

/// The priority class of the current process.
pub fn class() -> Result<ProcessClass, io::Error> {
    // SAFETY: no precondition; returns a pseudo-handle that needs no closing.
    class_of(unsafe { GetCurrentProcess() })
}

/// The priority class of the process behind `handle`, which needs
/// `PROCESS_QUERY_LIMITED_INFORMATION` access.
///
/// Not `unsafe`, for the same reason as the functions in
/// [`crate::thread::query`]: an invalid handle makes Windows return an error.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn class_of(handle: HANDLE) -> Result<ProcessClass, io::Error> {
    // SAFETY: the handle is only passed to the kernel, which validates it.
    let raw = unsafe { GetPriorityClass(handle) };
    if raw == 0 {
        return Err(io::Error::last_os_error());
    }
    ProcessClass::from_win32(raw).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown priority class {raw:#x}"),
        )
    })
}

#[cfg(test)]
#[path = "tests/class.rs"]
mod class_test;
