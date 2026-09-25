//! Fixed-capacity ring buffer: power-of-two storage, mask-based indexing.
//! `push_*` overwrites the opposite end when full, `try_push_*` refuses instead.
//!
//! - [`RingBuffer`]: the ring itself.
//! - [`RingCursor`]: its index arithmetic without storage, to drive a
//!   struct-of-arrays ring.
//! - [`ring_statistics`]: rolling mean and variance in O(1) per push.
//!
//! # Performance vs `VecDeque` (bounded emulation)
//!
//! Measured with `cargo +nightly bench -p ring_buffer --features nightly-benches`,
//! five runs, throughput-normalised (see `benches/`). Ratios above 1 mean this
//! crate is faster.
//!
//! | Operation                          | Ratio vs `VecDeque`                        |
//! |------------------------------------|--------------------------------------------|
//! | `push_tail` when full (overwrite)  | **x2.9**, constant from 512 B to 512 KB    |
//! | `push_head` when full              | x2.0, constant                             |
//! | Rotation front to back             | x1.25 (the trading engine's access pattern) |
//! | Rotation back to front             | x0.9 (cost of the materialised `tail`)     |
//! | Sequential read (`as_slices`)      | x0.8 to 1.0, memory-bound, run-dependent   |
//! | `try_push` rejection when full     | ~0.9 ns/op                                 |
//!
//! Details. Times are libtest's ns/iter (lower is better); the ratio is the
//! ring's average throughput over `VecDeque`'s, from libtest's MB/s figures,
//! so above 1 the ring is faster.
//!
//! | Family                 | Capacity | Ring avg | Std avg | Ring best | Std best | Ratio |
//! |------------------------|---------:|---------:|--------:|----------:|---------:|------:|
//! | `iter_sum`             | 64       | 16       | 13      | 15        | 12       | 0.80  |
//! | `iter_sum`             | 1 000    | 221      | 195     | 212       | 178      | 0.89  |
//! | `iter_sum`             | 1 024    | 236      | 199     | 214       | 173      | 0.83  |
//! | `iter_sum`             | 1 025    | 393      | 193     | 385       | 175      | 0.96  |
//! | `iter_sum`             | 48 000   | 12 998   | 8 334   | 12 262    | 7 987    | 0.88  |
//! | `iter_sum`             | 65 536   | 13 472   | 11 586  | 12 327    | 10 978   | 0.87  |
//! | `push_tail`            | 64       | 172      | 391     | 137       | 385      | 2.40  |
//! | `push_tail`            | 1 000    | 2 314    | 6 119   | 2 091     | 6 054    | 2.71  |
//! | `push_tail`            | 1 024    | 2 217    | 6 200   | 2 141     | 6 147    | 2.80  |
//! | `push_tail`            | 1 025    | 2 292    | 6 676   | 2 143     | 6 153    | 2.90  |
//! | `push_tail`            | 48 000   | 110 718  | 343 952 | 100 600   | 288 168  | 3.01  |
//! | `push_tail`            | 65 536   | 148 678  | 608 761 | 137 415   | 406 899  | 3.83  |
//! | `push_head`            | 64       | 152      | 273     | 138       | 269      | 1.83  |
//! | `push_head`            | 1 000    | 2 113    | 4 229   | 2 092     | 4 212    | 2.00  |
//! | `push_head`            | 1 024    | 2 288    | 4 418   | 2 148     | 4 307    | 1.94  |
//! | `push_head`            | 1 025    | 2 318    | 4 320   | 2 143     | 4 309    | 1.90  |
//! | `push_head`            | 48 000   | 100 489  | 202 491 | 100 410   | 201 938  | 2.02  |
//! | `push_head`            | 65 536   | 175 506  | 281 542 | 137 275   | 277 143  | 1.66  |
//! | `rotate_bf`            | 64       | 167      | 155     | 167       | 150      | 0.93  |
//! | `rotate_bf`            | 1 000    | 2 517    | 2 499   | 2 513     | 2 277    | 0.97  |
//! | `rotate_bf`            | 1 024    | 3 130    | 2 476   | 2 573     | 2 328    | 0.81  |
//! | `rotate_bf`            | 1 025    | 2 881    | 2 332   | 2 635     | 2 330    | 0.83  |
//! | `rotate_bf`            | 48 000   | 140 906  | 121 823 | 121 521   | 109 450  | 0.87  |
//! | `rotate_bf`            | 65 536   | 165 238  | 150 245 | 164 895   | 149 423  | 0.91  |
//! | `rotate_fb`            | 64       | 183      | 231     | 165       | 206      | 1.26  |
//! | `rotate_fb`            | 1 000    | 2 676    | 3 153   | 2 510     | 3 146    | 1.19  |
//! | `rotate_fb`            | 1 024    | 2 658    | 3 592   | 2 574     | 3 227    | 1.32  |
//! | `rotate_fb`            | 1 025    | 2 579    | 3 555   | 2 578     | 3 224    | 1.34  |
//! | `rotate_fb`            | 48 000   | 121 279  | 166 982 | 121 120   | 150 790  | 1.34  |
//! | `rotate_fb`            | 65 536   | 165 001  | 212 933 | 164 483   | 205 940  | 1.29  |
//! | `try_push_full_reject` | 1 024    | 885      | n/a     | 880       | n/a      | ring only |
//! | `try_push_pop_pair`    | 1 024    | 1 833    | n/a     | 1 716     | n/a      | ring only |

mod error;
mod ring_buffer;
mod ring_cursor;
pub mod ring_statistics;

pub use error::InitError;
pub use ring_buffer::{DrainRingFront, RingBuffer};
pub use ring_cursor::RingCursor;
