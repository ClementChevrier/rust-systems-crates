# wincore

`std` lets you start threads and allocate memory. It does not let you ask what
happened next: how much CPU a thread actually consumed, which core it ended up
on, what priority it was really given, how much memory the process holds, how
many page faults it took, or whether any of the settings you asked for were
applied at all.

Instrumentation added inside the program does not fill that gap either. It only
measures what the code was awake to measure: a thread that gets descheduled, or
a page that gets evicted, cannot report it.

`wincore` is the outside view. It asks Windows directly, for threads, for the
process, and for the pages underneath. Hand-written kernel32 FFI, no binding
crate, which keeps the surface to the twenty-odd calls I need.

## Layers

The dependency goes one way only.

`thread`, `process` and `mem` are stateless, each function wrapping one kernel32
call.

- `thread::Builder`: `std::thread::Builder` plus pinning, Win32 priority and
  boost policy. Settings are applied inside the new thread before its closure
  starts, and `spawn` blocks until that is done, so you never observe a
  half-configured thread. A setting that fails aborts neither the spawn nor the
  others: you get the thread and a `ConfigReport` saying what took effect.
  Degraded placement beats a missing worker.
- `thread::query`: read the real state back. CPU time, cycles, priority, boost,
  affinity, ideal processor, alive or not.
- `mem::lock`: pin a buffer's pages into RAM so touching it never costs a hard
  page fault. RAII guard, page granularity, and the documentation says why that
  matters. Pass a slice, not a container: this locks the pages of the value you
  hand it, and a `Vec`'s value is its 24-byte header.
- `process`: priority class, memory counters, working set limits. Raising the
  working set minimum is what makes locking anything larger than about 1.4 MB
  possible in the first place.

`monitor` is the only stateful layer: a process-wide registry of watched
threads, and `Snapshot::capture()` asking Windows about each. It keeps its own
registry rather than enumerating the process's threads because it also records
intent, what you asked for at spawn, so each thread's report shows intent and
reality side by side.

`Snapshot::ascii_report()` renders the whole thing as a fixed-width console
table. It is there to look nice, since I have my own interface for the real
thing. Every field in it is a `Result`, so a query Windows refuses shows a dash
with the reason on the line below.

## Used by the benchmarks next door

`channel`'s latency harness pins its producer and consumer threads with
`thread::Builder`, sets them to `TimeCritical` with the boost policy disabled so
the scheduler is not measured instead of the queue, raises the working set with
`process::modify_working_set_limits`, and locks its sample buffer with
`mem::lock_mut` so a page fault cannot land in the middle of a measurement.

## Windows only

`compile_error!` on any other target. No fallback, and none intended: the point
is precisely the things `std` does not expose.

## Known limitations

- A thread joins the registry only by enrolling itself or being enrolled from
  its `JoinHandle`. Dead entries are dropped by `purge_dead()` or at the next
  capture.
- `Snapshot::capture` holds the registry lock for the whole read. That is the
  price of a consistent picture and it is off the critical path.
