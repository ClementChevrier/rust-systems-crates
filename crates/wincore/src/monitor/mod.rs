//! Watching the process over time.
//!
//! The only layer in the crate that keeps state: [`watch_current`] and
//! [`watch`] enrol a thread in a process-wide registry, and
//! [`Snapshot::capture`] asks Windows about every thread listed there, and
//! about the process.
//!
//! A thread joins the registry only when it is enrolled: threads created
//! outside this crate, the main thread and the logger thread among them, have
//! to be enrolled by hand.
//!
//! The reason to keep an own registry rather than enumerate the process's
//! threads is that it also records *intent* ([`ThreadConfig`]): each
//! [`ThreadReport`] carries what was asked for next to what Windows reports
//! ([`ThreadState`]), so a worker that is no longer pinned where it should be
//! shows up side by side with its configuration.

mod process;
mod snapshot;
mod threads;

pub use process::ProcessReport;
pub use snapshot::*;
pub use threads::*;
