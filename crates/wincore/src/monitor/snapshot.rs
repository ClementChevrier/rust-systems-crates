use std::{fmt::Write, time::Instant};

use super::process::*;
use super::threads::*;
use crate::fmt::{bytes, cycles, duration};
use crate::thread::affinity::ThreadAffinity;

/// The process and every enrolled thread, as Windows reported them at one point.
#[derive(Debug)]
pub struct Snapshot {
    /// When the capture finished.
    pub timestamp: Instant,
    /// Process-wide state.
    pub process: ProcessReport,
    /// One report per enrolled thread, in thread id order.
    pub threads: Vec<ThreadReport>,
}
impl Snapshot {
    /// Queries Windows about the process and about every enrolled thread.
    ///
    /// The registry lock is held for the whole read: a thread enrolling or leaving
    /// during a snapshot waits until it is done. That is the price of a consistent
    /// picture, and it is off the critical path.
    pub fn capture() -> Self {
        let mut registry = registry_lock();
        let mut threads = Vec::with_capacity(registry.len());

        // Dead threads appear in this capture, then leave the registry.
        for index in (0..registry.len()).rev() {
            let report = ThreadReport::capture(&registry[index]);
            if matches!(report.alive, Ok(false)) {
                registry.swap_remove(index);
            }
            threads.push(report);
        }
        threads.reverse();
        drop(registry);

        Self {
            timestamp: Instant::now(),
            process: ProcessReport::capture(),
            threads,
        }
    }

    /// A fixed-width console report.
    ///
    /// Box-drawing characters, so the terminal has to be UTF-8 (Windows
    /// Terminal is; `cmd.exe` needs `chcp 65001`).
    pub fn ascii_report(&self) -> String {
        let mut out = String::new();

        let watched = self.threads.len();
        let exited = self
            .threads
            .iter()
            .filter(|r| matches!(r.alive, Ok(false)))
            .count();
        let age = duration(self.timestamp.elapsed());

        // Writing into a String cannot fail, so nothing is threaded through.
        let _ = writeln!(
            out,
            "╔══════════════════════════════════════════════════════════════════════════════════════════════════╗"
        );
        let _ = writeln!(
            out,
            "║ {:<76}{:>20} ║",
            "RUNTIME SNAPSHOT",
            format!("captured {age} ago")
        );
        let _ = writeln!(
            out,
            "╠══════════════════════════════════════════════════════════════════════════════════════════════════╣"
        );
        let _ = writeln!(out, "║ {:<96} ║", "PROCESS");

        match &self.process.prio {
            Ok(class) => {
                let _ = writeln!(
                    out,
                    "║   {:·<28}{:>12}{:55}║",
                    "Priority class ",
                    class.to_string(),
                    ""
                );
            }
            Err(source) => {
                let _ = writeln!(
                    out,
                    "║   {:·<28}{:<65.65}  ║",
                    "Priority class ",
                    format!("unreadable: {source}")
                );
            }
        }

        match &self.process.memory {
            Ok(memory) => {
                let _ = writeln!(
                    out,
                    "║   {:·<28}{:>12}   {:·<28}{:>12}{:12}║",
                    "Working set ",
                    bytes(memory.ram_use),
                    "Peak ",
                    bytes(memory.max_ram_used),
                    ""
                );
                let _ = writeln!(
                    out,
                    "║   {:·<28}{:>12}   {:·<28}{:>12}{:12}║",
                    "Private working set ",
                    bytes(memory.private_ram),
                    "Private committed ",
                    bytes(memory.private_mem),
                    ""
                );
                let _ = writeln!(
                    out,
                    "║   {:·<28}{:>12}   {:·<28}{:>12}{:12}║",
                    "Pagefile ",
                    bytes(memory.page_file_used),
                    "Pagefile peak ",
                    bytes(memory.max_page_file_used),
                    ""
                );
                let _ = writeln!(
                    out,
                    "║   {:·<28}{:>12}{:55}║",
                    "Page faults ", memory.page_fault, ""
                );
            }
            Err(source) => {
                let _ = writeln!(
                    out,
                    "║   {:·<28}{:<65.65}  ║",
                    "Memory ",
                    format!("unreadable: {source}")
                );
            }
        }

        match &self.process.memory_limits {
            Ok(limits) => {
                let _ = writeln!(
                    out,
                    "║   {:·<28}{:>12}   {:·<28}{:>12}{:12}║",
                    "Working set min ",
                    bytes(limits.minimum),
                    "Working set max ",
                    bytes(limits.maximum),
                    ""
                );
                if !limits.flags.is_empty() {
                    let flags = limits
                        .flags
                        .iter()
                        .map(|f| f.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(out, "║   {:·<28}{:<65.65}  ║", "Working set flags ", flags);
                }
            }
            Err(source) => {
                let _ = writeln!(
                    out,
                    "║   {:·<28}{:<65.65}  ║",
                    "Working set limits ",
                    format!("unreadable: {source}")
                );
            }
        }

        let _ = writeln!(
            out,
            "╠══════════════════════════════════════════════════════════════════════════════════════════════════╣"
        );
        let _ = writeln!(
            out,
            "║ {:<96} ║",
            format!("THREADS   {watched} watched · {exited} exited")
        );
        let _ = writeln!(out, "║{:98}║", "");

        // Header, rule and data rows all use this same format string — only the
        // `.19` truncation on the name differs. That is what makes a column
        // impossible to misalign.
        let _ = writeln!(
            out,
            "║   {:<8}  {:<19}  {:<12}  {:<5}  {:>10}  {:>9}  {:>9}  {:<6}   ║",
            "TID", "NAME", "PRIORITY", "BOOST", "CPU TIME", "CYCLES", "AFFINITY", "STATE"
        );
        let _ = writeln!(
            out,
            "║   {:<8}  {:<19}  {:<12}  {:<5}  {:>10}  {:>9}  {:>9}  {:<6}   ║",
            "────────",
            "───────────────────",
            "────────────",
            "─────",
            "──────────",
            "─────────",
            "─────────",
            "──────"
        );

        for report in &self.threads {
            let observed = &report.observed;

            let prio = match &observed.priority {
                Ok(value) => value.to_string(),
                Err(_) => UNREADABLE.to_string(),
            };
            let boost = match &observed.priority_boost {
                Ok(value) => value.to_string(),
                Err(_) => UNREADABLE.to_string(),
            };
            let cpu = match &observed.times {
                Ok(value) => duration(value.cpu()),
                Err(_) => UNREADABLE.to_string(),
            };
            let cycle = match &observed.cycle_count {
                Ok(value) => cycles(*value),
                Err(_) => UNREADABLE.to_string(),
            };
            let affinity = match &observed.affinity {
                Ok(value) => affinity_cell(value),
                Err(_) => UNREADABLE.to_string(),
            };
            let state = match report.alive {
                Ok(true) => "alive",
                Ok(false) => "exited",
                Err(_) => UNREADABLE,
            };

            let _ = writeln!(
                out,
                "║   {:<8}  {:<19.19}  {:<12}  {:<5}  {:>10}  {:>9}  {:>9}  {:<6}   ║",
                report.tid, report.name, prio, boost, cpu, cycle, affinity, state
            );

            // A dash says a cell is unreadable; these say why.
            if let Err(source) = &report.alive {
                let _ = writeln!(
                    out,
                    "║          └ {:<83.83}   ║",
                    format!("alive: {source}")
                );
            }
            if let Err(source) = &observed.times {
                let _ = writeln!(
                    out,
                    "║          └ {:<83.83}   ║",
                    format!("times: {source}")
                );
            }
            if let Err(source) = &observed.cycle_count {
                let _ = writeln!(
                    out,
                    "║          └ {:<83.83}   ║",
                    format!("cycle_count: {source}")
                );
            }
            if let Err(source) = &observed.priority {
                let _ = writeln!(
                    out,
                    "║          └ {:<83.83}   ║",
                    format!("priority: {source}")
                );
            }
            if let Err(source) = &observed.priority_boost {
                let _ = writeln!(
                    out,
                    "║          └ {:<83.83}   ║",
                    format!("priority_boost: {source}")
                );
            }
            if let Err(source) = &observed.affinity {
                let _ = writeln!(
                    out,
                    "║          └ {:<83.83}   ║",
                    format!("affinity: {source}")
                );
            }
            if let Err(source) = &observed.ideal_processor {
                let _ = writeln!(
                    out,
                    "║          └ {:<83.83}   ║",
                    format!("ideal_processor: {source}")
                );
            }
        }

        let _ = write!(
            out,
            "╚══════════════════════════════════════════════════════════════════════════════════════════════════╝"
        );
        out
    }
}

const UNREADABLE: &str = "—";

/// Width of the AFFINITY column.
const AFFINITY_WIDTH: usize = 9;

/// The affinity's `Display` when it fits the column, the CPU count otherwise:
/// a list of every CPU would break the fixed-width table.
fn affinity_cell(affinity: &ThreadAffinity) -> String {
    let full = affinity.to_string();
    if full.chars().count() <= AFFINITY_WIDTH {
        full
    } else {
        format!("{} CPUs", affinity.mask.count_ones())
    }
}

#[cfg(test)]
#[path = "tests/snapshot.rs"]
mod snapshot_test;
