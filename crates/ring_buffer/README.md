# ring_buffer

A fixed-capacity ring, O(1) rolling mean and standard deviation on top of it,
and a cursor you can drive your own struct-of-arrays ring with.

## The cursor is the point

The index arithmetic lives in `RingCursor`: head, length, capacity, mask, and no
`unsafe` and no storage at all. `RingBuffer<T>` is that cursor plus one
`Box<[MaybeUninit<T>]>`.

It is public so the same arithmetic can drive a struct-of-arrays ring: one
storage slice per field, all indexed by the same cursor.

## O(1) rolling statistics

`RingStatistics` keeps the mean and standard deviation of the window in O(1): a
push adds the new value to the accumulator and subtracts the evicted one.

Two accumulators, because incremental statistics lose precision when you
subtract values you added long ago:

- `LossyAccumulator`, running sum and sum of squares. Exact on integers, and it
  reports its own health: `Overflow` when a saturating operation clamped,
  `Unstable` when the cancellation ratio says the `f64` result is no longer
  trustworthy. It tells you when to stop believing it instead of quietly
  returning noise.
- `WelfordAccumulator`, numerically stable, slower.

`Scalar` is implemented for `i64`, `i128`, `u64`, `u128`, `f32` and `f64`, and
the documentation carries the table of which accumulator width each sample type
needs: the accumulator has to be wider than the samples, and `f64` stops being
exact past 53 bits of significand.

## The ring

Push and pop at both ends, three semantics each: `try_push_*` refuses when full,
`push_*` overwrites the opposite end, `push_*_evicting` returns what it evicted.
`as_slices()` gives the live window with no copy. `drain_from_head()` is lazy and
returns a guard, so using the buffer while draining is a compile error, and
abandoning it leaves the rest in the ring.

Capacity rounds up to a power of two, with both numbers visible through
`asked_capacity()` and `real_size()`. That is the opposite of `channel`, where a
non-power of two is a compile error, and the difference is deliberate: a channel
capacity is a const generic typed as a literal, a ring capacity usually comes
from an indicator period like a 200-bar average, and refusing it would push the
same rounding onto every caller.

## Against `VecDeque`

Best of five runs, throughput-normalised. Ratios above 1 mean this crate is
faster. Full per-capacity tables in the crate documentation.

| Operation | vs `VecDeque` |
|---|---|
| `push_tail` when full, overwriting | **x2.9**, constant from 512 B to 512 KB |
| `push_head` when full | x2.0 |
| Rotation front to back, the engine's access pattern | x1.25 |
| Sequential read through `as_slices` | x0.8 to 1.0, memory-bound |

The read being at or below parity is the honest line: once memory-bound,
`VecDeque` walks the same bytes and there is nothing left to win.
