//! Asynchronous buffered file logger: one background thread owns the file,
//! callers push pre-formatted messages through a bounded channel.
//!
//! The caller never blocks and never touches the disk. When the channel is full
//! the message is dropped and counted, and that count is written to the log as
//! soon as there is room again: loss is visible, never silent. One global
//! instance per process.
//!
//! ```no_run
//! use logger::{Logger, log_info};
//!
//! let mut logger = Logger::builder().output("./logs", "engine").build()?;
//! log_info!("FEED", "connected to {} in {} ms", "venue-a", 42);
//! logger.shutdown();
//! # Ok::<(), logger::Error>(())
//! ```
//!
//! # File management
//!
//! This crate owns writing and rotation: it creates the log directory, opens the
//! current file, rotates on `max_size` and on the UTC day boundary, and resumes
//! numbering after a restart.
//!
//! It deliberately does not own retention. Deleting, archiving and compressing
//! old files is left to an external process, so the log directory grows without
//! bound unless something purges it. Size the disk with
//! `max_size` x files per day x retention days.
//!
//! # Operating it while it runs
//!
//! The minimum level can be changed without a restart through the file passed to
//! [`LoggerBuilder::runtime_level_file`], polled at the interval set with
//! [`LoggerBuilder::poll_interval`]. [`Logger::shutdown`] is idempotent and
//! also runs on drop; it gives up after a timeout and detaches the worker
//! rather than blocking the whole process on a stalled disk, reporting on
//! stderr what was lost.
//!
//! Errors raised by the logger itself go to stderr: a logger must never crash
//! the application it observes, and it cannot log its own failure to write.
//!
//! # Known limitations
//!
//! - No inter-process lock. Two processes with the same folder and base name
//!   compute the same file number and append to the same file, and a 64 KiB
//!   `write_all` is not atomic on Windows, so their lines can interleave
//!   mid-line. Use one logger process per log directory.
//! - One logger per process: a second [`LoggerBuilder::build`], including after
//!   a shutdown, returns [`Error::AlreadyInitialized`].
//! - On restart, the file number is recovered from the file names, and a
//!   file's content is trusted as is: a file truncated by a crash is appended
//!   to, not repaired.

mod builder;
mod config;
mod error;
mod file_manager;
mod logger;
mod macros;
mod message;
pub mod metrics;
mod worker;

pub use builder::LoggerBuilder;
pub use config::message::TITLE_WIDTH;
pub use error::Error;
pub use file_manager::SyncPolicy;
pub use logger::Logger;
pub use macros::*;
pub use message::LogLevel;

/// Support for the macros. Not part of the API: do not use it directly.
#[doc(hidden)]
pub mod __private {
    pub use crate::config::message::TITLE_WIDTH;
    pub use crate::logger::{allowed, log};
}

#[cfg(test)]
#[path = "tests/lifecycle.rs"]
mod test;
