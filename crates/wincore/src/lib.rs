//! Low-level Win32 primitives for the live engine: configured thread creation,
//! reading a thread's real state, locking pages into RAM, and thread monitoring.
//!
//! The crate is two layers, and the dependency only ever goes one way:
//!
//! - [`thread`], [`process`] and [`mem`] are stateless. Each function wraps one
//!   kernel32 call, keeps nothing between calls, and can be used on its own.
//! - [`monitor`] is the only stateful layer: it keeps a process-wide registry of
//!   watched threads and captures snapshots of them. It builds on the other three,
//!   never the reverse.
//!
//! Windows only. The kernel32 symbols are declared by hand rather than pulled from
//! a binding crate, following the project's std-first rule.
//!
//! # Effective priority
//!
//! A thread's base priority combines the process class
//! ([`process::ProcessClass`]) and the thread level
//! ([`thread::priority::ThreadPriority`]). The full table is in the
//! [`thread::priority`] module docs.

#[cfg(not(windows))]
compile_error!("wincore only targets Windows (kernel32 FFI). No fallback is provided.");

mod ffi;
mod fmt;

pub mod mem;
pub mod monitor;
pub mod process;
pub mod thread;
