use std::{ffi::c_void, fmt::Write, io};

use crate::{
    ffi::{
        GetCurrentProcess, GetProcessWorkingSetSizeEx, K32GetProcessMemoryInfo,
        SetProcessWorkingSetSizeEx,
    },
    fmt::bytes,
};

/// `PROCESS_MEMORY_COUNTERS_EX2`, field for field.
#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
    private_working_set_size: usize,
    shared_commit_usage: u64,
}
impl ProcessMemoryCounters {
    fn new() -> Self {
        Self {
            cb: std::mem::size_of::<Self>() as u32,
            ..Default::default()
        }
    }
}

/// Memory counters of the current process, from `GetProcessMemoryInfo`.
/// Sizes are in bytes.
#[derive(Debug, Clone, Copy)]
pub struct MemoryStats {
    /// Page faults since the process started, soft and hard.
    pub page_fault: u32,

    /// Working set: the bytes currently resident in RAM.
    pub ram_use: usize,
    /// Peak working set.
    pub max_ram_used: usize,

    /// The part of the working set not shared with other processes.
    pub private_ram: usize,
    /// Private committed memory (Task Manager's "commit size").
    pub private_mem: usize,

    /// Committed memory backed by the page file.
    pub page_file_used: usize,
    /// Peak of `page_file_used`.
    pub max_page_file_used: usize,
}

impl MemoryStats {
    /// A fixed-width console report of these counters.
    pub fn ascii_report(&self) -> String {
        let mut out = String::new();
        // Writing into a String cannot fail, so the results are ignored.
        let _ = writeln!(out, "╔════════════════════════════════════════════════╗");
        let _ = writeln!(out, "║                 MEMORY RECAP                   ║");
        let _ = writeln!(out, "╠════════════════════════════════════════════════╣");
        let _ = writeln!(out, "║ RAM · global                                   ║");
        let _ = writeln!(
            out,
            "║   RAM ····························{:>12} ║",
            bytes(self.ram_use)
        );
        let _ = writeln!(
            out,
            "║   Peak RAM ·······················{:>12} ║",
            bytes(self.max_ram_used)
        );
        let _ = writeln!(out, "║                                                ║");
        let _ = writeln!(out, "║ RAM · private                                  ║");
        let _ = writeln!(
            out,
            "║   Working RAM ····················{:>12} ║",
            bytes(self.private_ram)
        );
        let _ = writeln!(
            out,
            "║   Reserved RAM ···················{:>12} ║",
            bytes(self.private_mem)
        );
        let _ = writeln!(out, "╠════════════════════════════════════════════════╣");
        let _ = writeln!(out, "║ PAGING                                         ║");
        let _ = writeln!(
            out,
            "║   Page faults ····················{:>12} ║",
            self.page_fault
        );
        let _ = writeln!(
            out,
            "║   Pagefile usage ·················{:>12} ║",
            bytes(self.page_file_used)
        );
        let _ = writeln!(
            out,
            "║   Peak pagefile ··················{:>12} ║",
            bytes(self.max_page_file_used)
        );
        let _ = write!(out, "╚════════════════════════════════════════════════╝");
        out
    }
}

impl From<ProcessMemoryCounters> for MemoryStats {
    fn from(value: ProcessMemoryCounters) -> Self {
        Self {
            page_fault: value.page_fault_count,

            ram_use: value.working_set_size,
            max_ram_used: value.peak_working_set_size,

            private_ram: value.private_working_set_size,
            private_mem: value.private_usage,

            page_file_used: value.pagefile_usage,
            max_page_file_used: value.peak_pagefile_usage,
        }
    }
}

/// Reads the memory counters of the current process.
pub fn memory() -> Result<MemoryStats, io::Error> {
    let mut info = ProcessMemoryCounters::new();
    // SAFETY: `info` is a live `PROCESS_MEMORY_COUNTERS_EX2` whose `cb` holds
    // its exact size, which is what the call writes at most.
    let rslt = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut info as *mut _ as *mut c_void,
            info.cb,
        )
    };

    if rslt == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(info.into())
    }
}

/// Whether the working-set limits are enforced (`QUOTA_LIMITS_HARDWS_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkingSetFlag {
    /// The minimum is a hint: pages can be trimmed below it under pressure.
    QuotaLimitsHardwsMinDisable,
    /// The minimum is enforced.
    QuotaLimitsHardwsMinEnable,
    /// The maximum is a hint: the working set can grow past it.
    QuotaLimitsHardwsMaxDisable,
    /// The maximum is enforced.
    QuotaLimitsHardwsMaxEnable,
}

impl WorkingSetFlag {
    fn as_win32(self) -> u32 {
        match self {
            Self::QuotaLimitsHardwsMinEnable => 1 << 0,
            Self::QuotaLimitsHardwsMinDisable => 1 << 1,
            Self::QuotaLimitsHardwsMaxEnable => 1 << 2,
            Self::QuotaLimitsHardwsMaxDisable => 1 << 3,
        }
    }

    const ALL: [Self; 4] = [
        Self::QuotaLimitsHardwsMinEnable,
        Self::QuotaLimitsHardwsMinDisable,
        Self::QuotaLimitsHardwsMaxEnable,
        Self::QuotaLimitsHardwsMaxDisable,
    ];

    fn from_mask(mask: u32) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|flag| mask & flag.as_win32() != 0)
            .collect()
    }

    fn to_mask(flags: &[WorkingSetFlag]) -> u32 {
        flags.iter().fold(0, |mask, flag| mask | flag.as_win32())
    }
}

impl std::fmt::Display for WorkingSetFlag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::QuotaLimitsHardwsMinEnable => "hard-min",
            Self::QuotaLimitsHardwsMinDisable => "soft-min",
            Self::QuotaLimitsHardwsMaxEnable => "hard-max",
            Self::QuotaLimitsHardwsMaxDisable => "soft-max",
        })
    }
}

/// Working-set limits of the current process, in bytes.
///
/// Raising `minimum` is what lets [`crate::mem::lock`] pin more than a few
/// pages: Windows caps the lockable memory at the minimum working set.
#[derive(Debug, Clone)]
pub struct WorkingSetLimits {
    /// Minimum working set.
    pub minimum: usize,
    /// Maximum working set.
    pub maximum: usize,
    /// How strictly each limit is enforced.
    pub flags: Vec<WorkingSetFlag>,
}

/// Reads the working-set limits of the current process.
pub fn working_set_limits() -> Result<WorkingSetLimits, io::Error> {
    let mut min: usize = 0;
    let mut max: usize = 0;
    let mut flags: u32 = 0;

    // SAFETY: the three out-pointers point at live locals of the right types.
    let rslt =
        unsafe { GetProcessWorkingSetSizeEx(GetCurrentProcess(), &mut min, &mut max, &mut flags) };

    if rslt == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(WorkingSetLimits {
            minimum: min,
            maximum: max,
            flags: WorkingSetFlag::from_mask(flags),
        })
    }
}

/// Sets the working-set limits of the current process.
///
/// Needs the "increase quota" privilege to raise the minimum past what
/// Windows grants by default.
pub fn modify_working_set_limits(new: WorkingSetLimits) -> Result<(), io::Error> {
    // SAFETY: plain values only; the process pseudo-handle needs no closing.
    let rslt = unsafe {
        SetProcessWorkingSetSizeEx(
            GetCurrentProcess(),
            new.minimum,
            new.maximum,
            WorkingSetFlag::to_mask(&new.flags),
        )
    };

    if rslt == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/memory.rs"]
mod memory_test;
