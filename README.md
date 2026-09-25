# rust-systems-crates

[![CI](https://github.com/ClementChevrier/rust-systems-crates/actions/workflows/ci.yml/badge.svg)](https://github.com/ClementChevrier/rust-systems-crates/actions/workflows/ci.yml)

Four crates from the runtime of a live trading engine I am building, plus one
development tool.

They share one rule: standard library first. A third-party crate is allowed only
when something is impossible with `std` alone, and the reason gets written down
before the code. None of these has a third-party dependency in production. The
only external names in `Cargo.lock` are `loom`, compiled only under `--cfg loom`,
and the reference implementations the benchmarks measure against.

The point is not purity. On a latency-sensitive path I want to be able to read
everything that runs between a market event and an order, and to know why each
atomic ordering is the one it is.

| Crate | What it is |
|---|---|
| [`channel`](crates/channel) | Bounded SPSC and MPSC queues, and a snapshot channel |
| [`ring_buffer`](crates/ring_buffer) | Fixed-capacity ring, O(1) rolling mean and standard deviation, SoA-friendly cursor |
| [`logger`](crates/logger) | Asynchronous file logger, one I/O thread, bounded and lossy by design |
| [`wincore`](crates/wincore) | Win32 thread creation, pinning, priority and monitoring, hand-written FFI |
| [`tasks`](crates/tasks) | A `task!` proc macro turning a `todo!()` into a tracked markdown file. Development tool, work in progress |

## Build

Windows only. `wincore` binds kernel32 directly with no fallback, and the
benchmark harnesses use it for pinning. The concepts map across without much
change: `sched_setaffinity`, `mlock`, `SCHED_FIFO`, `perf_event_open`,
`isolcpus`. `channel`, `ring_buffer` and `tasks` are portable as they stand.

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# loom explores interleavings a stress test can only sample
$env:RUSTFLAGS="--cfg loom"; $env:LOOM_MAX_PREEMPTIONS="3"
cargo test -p channel --release
Remove-Item Env:\LOOM_MAX_PREEMPTIONS; Remove-Item Env:\RUSTFLAGS

# Miri covers the batch memcpy paths
cargo +nightly miri test -p channel -p ring_buffer
```

## License

All rights reserved. Published to be read, not used. See [LICENSE](LICENSE).
