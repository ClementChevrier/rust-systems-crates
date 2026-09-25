//! Bounded wait-free SPSC: one producer, one consumer, capacity fixed at
//! compile time (const generic, power of two).
//!
//! # Why this module exists
//!
//! - `std::sync::mpsc` is a general-purpose MPSC: it pays for multi-producer
//!   support and blocking/parking that a 1-to-1 pipeline does not need.
//!   Measured here at ~73 ns/msg with `try_send`/`try_recv` against ~12 ns/msg
//!   for this module (~6x), and ~30x in batch mode.
//! - External equivalents (rtrb, ringbuf) are solid references, but this
//!   project is std-first: no third-party dependency in production.
//! - The hot path performs no allocation, no locking, no syscall. Explicit
//!   backpressure (`ChannelFull` returns the value), closure observable from
//!   both ends (`is_closed`), bounded, panic-safe draining.
//!
//! # Validation
//!
//! - **loom**: exhaustive interleaving exploration (publication, slot reuse at
//!   the full/empty boundary, closure, residue drop).
//! - **miri**: UB, provenance and leaks; covers the batch `memcpy` paths.
//! - **benches**: `cargo +nightly bench -p channel --bench spsc_compare --features "nightly-benches rtrb"`.
//!
//! # Metrics
//!
//! 2026-07, Windows 10 x86_64, two pinned physical cores, median of five runs.
//! Orders of magnitude: re-measure on the target machine.
//!
//! Cross-thread, `u64`, capacity 1024, 1M messages:
//!
//! | scenario                         | ns/msg | throughput                |
//! |----------------------------------|--------|---------------------------|
//! | `push` / `pop`                   | ~12    | ~86 M msg/s               |
//! | `push_batch` / `pop_batch` (64)  | ~2.4   | ~410 M msg/s              |
//! | 64 B payload (one cache line)    | ~21    | ~48 M msg/s (~3 GB/s)     |
//! | same-thread round trip           | ~9     | n/a                       |
//!
//! References, same machine, same protocol:
//!
//! | solution                                   | ns/msg                    |
//! |--------------------------------------------|---------------------------|
//! | `std::sync::mpsc` `try_send`/`try_recv`    | ~73                       |
//! | `std::sync::mpsc` `send`/`recv` (blocking) | ~80 to 150                |
//! | `rtrb 0.3` `push`/`pop`                    | ~9 to 19, machine-dependent |
//!
//! Mechanical cost per element (same thread, emptying a ring of 65,536 `u64`,
//! refill included), which isolates the method cost from coherence traffic:
//!
//! | method          | ns/element | note                                                   |
//! |-----------------|------------|--------------------------------------------------------|
//! | `pop` / `drain` | ~1.1       | one release store per element                          |
//! | `pop_batch`     | ~0.5       | `memcpy`-bound (~17 GB/s); no gain past 64 per batch   |
//!
//! The ~5x gap between same-thread (17 GB/s) and cross-thread (3 GB/s) is the
//! cost of inter-core cache coherence: a physical bound, not the code.
//!
//! # Which method for which job
//!
//! | need                                         | method                                  |
//! |----------------------------------------------|-----------------------------------------|
//! | lowest per-message latency                   | `push` / `pop`                          |
//! | maximum throughput, `T: Copy` (market data)  | `push_batch` / `pop_batch`              |
//! | batch of non-`Copy` items, single publish    | `push_iter`                             |
//! | decide without consuming                     | `peek`                                  |
//! | drain what is published, `Drop` types        | `drain` (bounded, panic-safe)           |
//! | zero-copy processing, `T: Copy`              | `read_chunk`, `as_slices`, `commit`     |
//! | discard stale data (resync)                  | `skip` / `clear`                        |
//! | clean shutdown                               | `is_closed()`, then `pop()` until `None` |
//!
//! When the producer finds the ring full, spinning or yielding is the caller's
//! choice: the waiting policy does not belong to the channel.

use super::sync;

mod queue;

pub use queue::{Consumer, Drain, Producer, ReadChunk, channel};
