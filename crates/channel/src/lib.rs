//! Bounded lock-free channels for a real-time pipeline: no allocation, no lock
//! and no syscall on the hot path, with explicit backpressure.
//!
//! Three channels, three jobs:
//!
//! - [`spsc`]: one producer, one consumer, wait-free, compile-time power-of-two
//!   capacity. Uniqueness is structural: neither end is `Clone`, and a
//!   `compile_fail` doctest proves it.
//! - [`mpsc`]: many producers, one consumer, global FIFO order, compile-time
//!   power-of-two capacity. Built on a ticket ring with a credit counter
//!   instead of a CAS retry loop.
//! - [`snapshot`]: one writer, one reader, latest value wins. The reader always
//!   has a valid value to look at and skips straight to the newest one.
//!
//! The two queues refuse rather than block when full: [`Error::ChannelFull`]
//! hands the value back, so the caller decides what to drop. Closure is
//! observable from either end of every channel.
//!
//! # Why not `std::sync::mpsc` or crossbeam
//!
//! `std::sync::mpsc` is unbounded by default and its bounded flavour locks under
//! contention; a market-feed hot path needs bounded memory and fail-fast
//! backpressure. crossbeam is a solid alternative, but this workspace is
//! std-first: no third-party crate in production. `rtrb`, `crossbeam-queue` and
//! `crossbeam-channel` remain optional bench baselines, never linked otherwise.
//!
//! Each module documents its own design, shutdown protocol and measurements;
//! start with [`mpsc`].
//!
//! # Validation
//!
//! ```text
//! # loom: exhaustive interleaving exploration, bounded to three preemptions
//! $env:RUSTFLAGS = "--cfg loom"; $env:LOOM_MAX_PREEMPTIONS = "3"
//! cargo test -p channel --release
//! Remove-Item Env:\LOOM_MAX_PREEMPTIONS; Remove-Item Env:\RUSTFLAGS
//!
//! # miri: undefined behaviour, provenance, leaks
//! cargo +nightly miri test -p channel
//!
//! # bench
//! cargo +nightly bench -p channel --bench mpsc_compare --features "nightly-benches crossbeam-queue crossbeam-channel"
//! ```

mod error;
mod sync;
mod util;

pub mod mpsc;
pub mod snapshot;
pub mod spsc;
pub use error::{Error, ReceiverDisconnected};
