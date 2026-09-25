# logger

The logger for my trading engine. Threads push messages into a bounded channel;
one background thread owns the file and does all the I/O. A worker on the hot
path never opens a file, never blocks on a lock, never waits for a slow disk.

## When it cannot keep up

It drops the message and counts it. An unbounded queue turns a slow disk into an
out-of-memory kill, and blocking the producer turns the logger into the thing
that stops the engine. The count is written to the log as soon as there is room
again, so the loss is visible and never silent.

The same rule everywhere, since a logger must never take down what it observes:
write failures go to stderr and the worker keeps running; stderr reporting is
rate limited, first failure then one in 32, because on a full disk stderr is
usually another file on the same volume; the payload is never echoed; a rotation
that cannot open the new file keeps writing to the current one.

## Operating it while it runs

The part I care most about, since the engine is meant to run unattended.

- **Change the level without a restart.** The worker polls a file you name, at
  an interval you pick.
- **Metrics footer on every rotation.** Messages in, writes, bytes, file size,
  plus the loss and degradation counters when they are non-zero. Each rotated
  file carries its own tally and reads on its own.
- **Readable from the program too.** `Logger::peek_metrics()` returns the same
  counters the footer prints, without resetting them, so a supervision endpoint
  can poll them. A rotation between two calls resets the window, so subtract
  with `saturating_sub` and treat a drop as "a rotation happened".
- **Shutdown that cannot hang the process.** Idempotent, also runs on drop. If
  the worker does not take the order within a timeout, a frozen disk for
  instance, the logger detaches it and reports what was lost rather than
  blocking on a join that will never return.
- **Sync policy you pick.** fsync on rotation only, or on every message at or
  above a level. Free in steady state, durable where it matters.

Rotation is `<folder>/<UTC date>/<base>_<n>.log`, on size and on the day
boundary, resuming the numbering after a restart.

## Known limitations

- No retention. This crate creates, writes and rotates; deleting and archiving
  is someone else's job, so the directory grows without bound. Size the disk
  with `max_size x files per day x retention days`.
- No inter-process lock. Two processes on the same folder and base name append
  to the same file, and a 64 KiB `write_all` is not atomic on Windows, so lines
  interleave mid-line. One logger process per log directory.
- One logger per process, since the sender lives in a `OnceLock`, which is what
  lets the log path take it without a lock. A second `build()`, including after
  a shutdown, returns `Error::AlreadyInitialized`.
- On a crash you lose the channel contents and the unflushed buffer, plus, under
  `SyncPolicy::OnRotation`, whatever the OS had not written yet.

## What the caller still pays

The I/O is off your thread; the formatting is not. An accepted message costs a
`format!` and one heap allocation on the calling thread. The level check comes
first, so a message below the threshold costs an atomic load and nothing else.

The fix is known: push the raw arguments as a compact binary record and format
on the I/O thread, the way NanoLog and Quill do. The static parts already
cooperate, since the title is a `&'static str` and the format string is a
literal at the macro site; the work is serialising each argument without going
back through `fmt::Arguments`, which borrows and cannot be stored. That is the
next change here.
