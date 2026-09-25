//! Creating and observing Windows threads.
//!
//! [`Builder`] extends `std::thread::Builder` with CPU pinning, Win32 priority and
//! the boost policy. Settings are applied *inside* the new thread before its
//! closure starts, and [`Builder::spawn`] blocks until that is done: the caller
//! never observes a half-configured thread.
//!
//! A setting that cannot be applied aborts neither the spawn nor the other
//! settings. The thread runs and [`ConfigReport`] says which ones took effect —
//! degraded placement beats a missing worker.
//!
//! [`query`] reads a thread's real state back from Windows; [`priority`],
//! [`affinity`] and [`times`] carry the matching types.

mod builder;
mod outcome;

pub mod affinity;
pub mod control;
pub mod priority;
pub mod query;
pub mod times;

pub use builder::Builder;
pub use outcome::*;
