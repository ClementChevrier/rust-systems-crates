# channel

Message passing for the path between a market event and an order.

`std::sync::mpsc` is unbounded by default and its bounded flavour locks under
contention. The ecosystem alternatives are good, but using one means trusting a
few thousand lines I have not read, for the one property I care about: what the
producer pays per message when the consumer is keeping up. So I wrote it.

## Numbers

Median of 33 passes across three sessions, four million messages each. Windows
10 x86_64, producers and consumer pinned to distinct physical cores, open-loop
pacing so a link that falls behind carries the delay in its own samples instead
of hiding it. Harness in `benches/latency.rs`.

Absolute values moved 10 to 15 percent between sessions and roughly 70 ns of
every figure is the clock read itself, so read the columns against each other
rather than as values.

**4M msg/s offered, one producer, nanoseconds one way:**

| | p50 | p90 | p99 | p99.9 |
|---|---|---|---|---|
| `channel::spsc` | 130.7 | **146.8** | **164.8** | **280.0** |
| `channel::mpsc` | **129.7** | 150.3 | 175.8 | 285.0 |
| crossbeam ArrayQueue | 160.3 | 182.3 | 204.4 | 325.1 |
| `std::sync::mpsc` | 159.8 | 181.3 | 204.4 | 335.1 |
| crossbeam-channel | 159.8 | 182.3 | 205.9 | 339.6 |

About 19 percent below the best alternative on p50 and p90, 14 on p99, holding
from 1M to 6M msg/s offered.

**It also keeps up when the others stop.** Every pass is checked against its own
schedule: one whose last tenth is more than four times heavier than its first
never reached steady state, and its quantiles measure backlog rather than
latency. Across 33 passes per configuration:

| passes that fell behind | `channel::mpsc` | ArrayQueue | crossbeam-channel |
|---|---|---|---|
| three producers | **0/33** | 3/33 | 1/33 |
| six producers, oversubscribed | **0/33** | 2/33 | 2/33 |
| 10M msg/s, one producer | **0/33** | 1/33 | 1/33 |

That is a count, not a quantile, so it needs no error bars.

## Three channels, three jobs

- `spsc`: one producer, one consumer, bounded, wait-free
- `mpsc`: many producers, one consumer, bounded, lock-free, global FIFO
- `snapshot`: one writer, one reader, latest wins, linked-list

## Design

- Capacity is a const generic, so the wrap-around is a mask and the buffer is
  one allocation made once. Non-powers of two are a compile error here, unlike
  `ring_buffer` which rounds up: the capacity is a literal here and a runtime
  value there.
- Cache-line padding between the two cursors, and cached cursors: the producer
  reloads the consumer's atomic only when its own copy says full.
- The MPSC is a ticket ring. A credit counter enforces the bound before a ticket
  is taken, which is what makes a fail-fast push possible at all, since a
  `fetch_add` ticket cannot be handed back. Per-cell sequence numbers carry the
  only two happens-before edges; everything else is Relaxed. The cost is two
  atomic read-modify-writes per push where a CAS ring pays one: no retry on the
  fast path, more coherence traffic as producers are added.
- Batch paths split the wrap into two contiguous slices so the compiler emits
  two `memcpy`, and publish with a single release store.
- `push_iter` publishes what it wrote even if the iterator panics, through a
  drop guard.

## Correctness

Every `unsafe` block carries a `// SAFETY:` comment that refers to a numbered
invariant stated at the top of its module, and
`clippy::undocumented_unsafe_blocks` keeps it that way. Beyond that: loom models for publication, slot reuse at the boundaries,
closure and residue drop; Miri for undefined behaviour, provenance and leaks;
`compile_fail` doctests pinning the API shape, such as the SPSC producer not
being `Clone`; compile-time `Send` and `Sync` assertions.

```powershell
# loom explores interleavings a stress test can only sample, bounded to three
# preemptions: past that the state space grows faster than the coverage does,
# and the spin loops in write_cell make every yield a branch point.
$env:RUSTFLAGS="--cfg loom"; $env:LOOM_MAX_PREEMPTIONS="3"
cargo test -p channel --release
Remove-Item Env:\LOOM_MAX_PREEMPTIONS; Remove-Item Env:\RUSTFLAGS

# Miri covers the batch memcpy paths
cargo +nightly miri test -p channel
```

## Known limitations

- Power-of-two capacity only.
- Sustained throughput is 15.6 M msg/s for the MPSC and 44.3 for the SPSC,
  falling to 10.8 with six producers. Queueing sets in past roughly half of
  that, so size for 7 M msg/s rather than 15.
- The lead narrows with producers, from about 15 percent at one to 10 at three.
  The slopes are within session-to-session variation of each other, so this does
  not support a claim that the wait-free ticket stays flat where a CAS ring
  degrades.
- No `read_chunk` on the MPSC side: cells interleave data and sequence number,
  and published items can have unpublished holes between them, so no contiguous
  `&[T]` can be produced soundly.
- `snapshot` is a linked list: one allocation per publish. For the routing
  table that is fine, since it changes when a strategy is added or removed,
  not per tick. The real exposure is a reader that stops calling `update()`:
  nodes then pile up, one per publish, and the writer cannot see it. A triple
  buffer, three slots allocated once with one atomic exchange each side,
  removes both and gives up only the intermediate versions, which this
  channel does not want anyway. That is the next change here.
