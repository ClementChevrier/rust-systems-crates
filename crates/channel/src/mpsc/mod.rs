//! Bounded lock-free MPSC: many producers, one consumer, global FIFO order,
//! capacity fixed at compile time (const generic, power of two).
//!
//! # Why this module exists
//!
//! - `std::sync::mpsc` is unbounded by default and pays for blocking/parking;
//!   its bounded flavor (`sync_channel`) locks under contention. A market-feed
//!   hot path needs bounded memory, fail-fast backpressure and no syscalls.
//! - External equivalents (crossbeam-channel, crossbeam-queue) are solid but
//!   this project is std-first: no third-party dependency in production. They
//!   remain as bench baselines (optional features, never linked in prod).
//! - Hot path performs no allocation, no locking, no CAS retry loop.
//!
//! # Design (ticket ring + credits)
//!
//! Three cooperating mechanisms, each owning one concern:
//!
//! - **`tail` (fetch_add)** hands out one unique ticket per push: the global
//!   FIFO order. Wait-free, so a push always succeeds on its first attempt,
//!   unlike the classic Vyukov queue (see `ref_vyukov`, kept as bench baseline)
//!   where a losing producer retries its CAS.
//! - **`tickets` (credit counter)** enforces the bound *before* the ticket is
//!   taken, which is what makes fail-fast `push` possible at all: a
//!   `fetch_add` ticket cannot be given back, so the reservation must happen
//!   first. Credits are fungible: they prove *a* slot is free, not that
//!   *your* slot is visible yet, hence the seq spin below.
//! - **`seq` (one per cell)** carries the only two happens-before edges:
//!   publication (producer `store(Release)` -> consumer `load(Acquire)`) and
//!   recycling (consumer `store(Release)` -> next-lap producer
//!   `load(Acquire)`). Everything else (`tail`, `tickets`) is Relaxed: pure
//!   counters, no data rides on them.
//!
//! The consumer is unique, so `head` lives as a plain field inside
//! `Consumer`: never shared, never atomic.
//!
//! The cost of that shape is two atomic read-modify-writes per push, the credit
//! and then the ticket, where a CAS ring pays one. That is the trade: no retry
//! on the fast path, more coherence traffic as producers are added. Both halves
//! show up in the measurements below.
//!
//! Reserved-but-unpublished slots are observable: `tail` moves before the
//! producer writes. `try_pop` surfaces this as `PopOutcome::Busy` (head slot
//! reserved by a producer that has not published yet, preempted or mid-write)
//! as opposed to `Empty`. `pop` collapses both to `None` for the common
//! polling loop. In practice that state is rare: see the note under Metrics.
//!
//! # Shutdown protocol
//!
//! Producer side: `push` fails with `ReceiverDisconnected` once the consumer
//! is dropped. Consumer side: `closed` is set by the *last* producer drop
//! (release/acquire chain), so the safe termination check is, in this order:
//!
//! ```text
//! if rx.is_closed() {            // 1. no producer left, no future push
//!     while let Some(v) = rx.pop() { ... }   // 2. drain what was published
//!     // this None is final
//! }
//! ```
//!
//! Checking `pop` before `is_closed` is racy: an item can be pushed and the
//! last producer can drop between your two checks.
//!
//! # Validation
//!
//! - **tests**: `cargo test -p channel`
//! - **loom** (interleaving exploration bounded to three preemptions, which is
//!   where the coverage is; publication edge, recycle edge at the full
//!   boundary, concurrent batch credit refunds, drain vs holes):
//!   `$env:RUSTFLAGS="--cfg loom"; $env:LOOM_MAX_PREEMPTIONS=3; cargo test -p channel --release`
//! - **miri** (UB, provenance, leaks):
//!   `cargo +nightly miri test -p channel`
//! - **latency distribution** (`benches/latency.rs`, stable toolchain):
//!   `cargo bench -p channel --bench latency --features "crossbeam-queue crossbeam-channel"`
//! - **throughput** (`benches/mpsc_compare.rs`, nightly):
//!   `cargo +nightly bench -p channel --bench mpsc_compare --features "nightly-benches crossbeam-queue crossbeam-channel"`
//!
//! # Metrics
//!
//! From `benches/latency.rs`: open-loop pacing at a fixed offered rate, so a
//! link that falls behind carries the delay in its own samples instead of
//! slowing the producer down and hiding it. Median of 33 passes across three
//! sessions, four million measured messages per pass. Windows 10 x86_64, `u64`
//! payload, capacity 1024, producers and consumer pinned.
//!
//! Absolute values moved 10 to 15 percent between sessions, and roughly 70 ns
//! of every figure is the clock read itself. **Read the columns against each
//! other, not as values.** Re-measure on the target machine.
//!
//! ## One producer, 4M msg/s offered (nanoseconds, one way)
//!
//! | link                    | p50   | p90   | p99   | p99.9 |
//! |-------------------------|-------|-------|-------|-------|
//! | this module             | 129.7 | 150.3 | 175.8 | 285.0 |
//! | `spsc` (for scale)      | 130.7 | 146.8 | 164.8 | 280.0 |
//! | crossbeam ArrayQueue    | 160.3 | 182.3 | 204.4 | 325.1 |
//! | `std::sync::mpsc`       | 159.8 | 181.3 | 204.4 | 335.1 |
//! | crossbeam-channel       | 159.8 | 182.3 | 205.9 | 339.6 |
//!
//! About 19 percent below the best alternative on p50 and p90, 14 percent on
//! p99, holding from 1M to 6M msg/s offered. Note that the MPSC costs almost
//! nothing against the SPSC at the median: the generalization is paid in the
//! tail, not in the common case.
//!
//! ## Adding producers, 4M msg/s offered in total (p99)
//!
//! | link                 | x1    | x2    | x3    |
//! |----------------------|-------|-------|-------|
//! | this module          | 174.8 | 185.8 | 209.4 |
//! | crossbeam ArrayQueue | 204.4 | 218.9 | 246.0 |
//! | `std::sync::mpsc`    | 204.4 | 237.9 | 233.9 |
//! | crossbeam-channel    | 204.9 | 241.0 | 241.5 |
//!
//! The lead narrows from about 15 percent to 10. The slopes across these four
//! are within session-to-session variation of each other, so this data does
//! **not** support a claim that the wait-free ticket keeps the cost flat where
//! a CAS ring degrades. It supports a lower floor, nothing more.
//!
//! ## Six producers oversubscribed onto three cores
//!
//! | link                 | p50   | p90   | p99   | p99.9 |
//! |----------------------|-------|-------|-------|-------|
//! | this module          | 141.3 | 164.3 | 203.4 | 537.0 |
//! | crossbeam ArrayQueue | 181.8 | 210.4 | 244.0 | 715.8 |
//! | crossbeam-channel    | 184.3 | 212.9 | 245.0 | 760.4 |
//! | `std::sync::mpsc`    | 185.8 | 212.9 | 249.0 | 651.7 |
//!
//! The hostile configuration, and the best relative result: ahead on every
//! column including p99.9.
//!
//! ## Keeping up with the offered load
//!
//! Every pass is checked against its own schedule: a pass whose last tenth is
//! more than four times heavier than its first tenth never reached steady
//! state, and its quantiles measure backlog rather than latency. Across 33
//! passes per configuration:
//!
//! | passes that fell behind | this module | ArrayQueue | crossbeam-channel |
//! |-------------------------|-------------|------------|-------------------|
//! | three producers         | 0/33        | 3/33       | 1/33              |
//! | six producers, oversub  | 0/33        | 2/33       | 2/33              |
//! | 10M msg/s, one producer | 0/33        | 1/33       | 1/33              |
//!
//! This is a count, not a quantile, so it does not need error bars. Under
//! contention the alternatives occasionally lose the rhythm entirely; this one
//! did not, in any configuration measured.
//!
//! ## Sustained throughput and where to size
//!
//! | producers | M msg/s |
//! |-----------|---------|
//! | 1         | 15.6    |
//! | 2         | 13.8    |
//! | 3         | 12.0    |
//! | 6         | 10.8    |
//!
//! (The SPSC does 44.3 for comparison.) Queueing sets in well before
//! saturation: p99 is flat from 1M to 6M offered, then 239 ns at 8M and 439 ns
//! at 10M. **Size for roughly half the sustained throughput**, so 7 M msg/s,
//! not 15.
//!
//! ## `Busy` is rare
//!
//! The harness counts every `try_pop` that returned `Busy`. Across all
//! configurations it lands between 29 and 557 occurrences per four million
//! messages, oversubscription included, and `push` was never refused under
//! paced load. So the reserved-but-unpublished window is real but tiny, and it
//! does not explain the tail. `Busy` earns its place as an honest signal to the
//! consumer, not as a hot path concern.
//!
//!
//! # Which method for which job
//!
//! | need                                     | method                          |
//! |------------------------------------------|---------------------------------|
//! | lowest per-message latency               | `push` / `pop`                  |
//! | burst of `T: Copy` (market data)         | `push_batch` (trusted length)   |
//! | batch of non-Copy types, moves items     | `push_iter` (a lying len() degrades to a shorter run, never wedges) |
//! | distinguish empty vs blocked-by-producer | `try_pop` (`Busy` vs `Empty`)   |
//! | decide without consuming                 | `peek`                          |
//! | drain published prefix, `Drop` types     | `drain` (stops at first hole)   |
//! | discard stale data (resync)              | `skip` / `clear`                |
//! | clean shutdown                           | `is_closed()` then drain (order matters, see above) |
//!
//! `read_chunk`/`as_slices` intentionally do not exist here: cells interleave
//! `data` and `seq` (AoS) and published items may have unpublished holes
//! between them, so no contiguous `&[T]` view can be soundly produced. Use
//! `drain` or `pop` in a loop; the SPSC module keeps the zero-copy chunk API.
//!
//! Producer full -> spin/yield is the caller's choice: the waiting policy does
//! not belong to the channel.
//!
//! # Future work
//!
//! - SoA layout (`data[N]` + `seq[N]` arrays) would enable memcpy batches and a
//!   chunk API bounded to the published prefix, at the cost of touching two
//!   cache lines per unit push. Likely a loss for one-at-a-time pushes, so it
//!   needs measuring before it is built.
//! - Block-based tail sharding (BBQ-style) only if `tail` fetch_add shows up as
//!   the bottleneck at target producer counts. Nothing in the current data
//!   points at it: `Busy` is negligible and the ring never fills under paced
//!   load, so the throughput decay from 15.6 to 10.8 M msg/s across six
//!   producers is unexplained and worth profiling before it is designed around.
//!   Sharding keeps FIFO per block, not globally, which is a real cost.
//! - Placement turns out to matter as much as the queue: two producers on the
//!   two hyper-threads of one core beat two producers on separate cores by 5 to
//!   10 percent, for every implementation tested. Worth revisiting how the
//!   worker pool assigns cores.

use super::sync;

mod queue;
pub use queue::{Consumer, Drain, PopOutcome, Producer, channel};

/// Bench-only baseline (classic Vyukov MPMC, CAS on both ends). Gated so it
/// never exists outside `--features nightly-benches` builds.
#[cfg(feature = "nightly-benches")]
pub mod ref_vyukov;

/// Bound on the consumer's publication wait before reporting `Busy`: the head
/// slot is reserved (tail moved) but its producer has not published yet.
/// Tiny under loom to keep the state space tractable.
#[cfg(not(loom))]
pub(super) const MAX_SPIN_BEFORE_FAIL: usize = 128;
#[cfg(loom)]
pub(super) const MAX_SPIN_BEFORE_FAIL: usize = 2;
