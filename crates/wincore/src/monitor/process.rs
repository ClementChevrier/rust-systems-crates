use std::io;

use crate::process::*;

/// The process-wide part of a [`Snapshot`](super::Snapshot). Each field is a
/// separate query, so one refusal does not hide the others.
#[derive(Debug)]
pub struct ProcessReport {
    /// Memory counters.
    pub memory: Result<MemoryStats, io::Error>,
    /// Working-set limits.
    pub memory_limits: Result<WorkingSetLimits, io::Error>,
    /// Priority class.
    pub prio: Result<ProcessClass, io::Error>,
}
impl ProcessReport {
    /// Queries Windows about the current process.
    pub fn capture() -> Self {
        Self {
            memory: memory(),
            memory_limits: working_set_limits(),
            prio: class(),
        }
    }
}
