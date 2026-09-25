# tasks

> Read this first. This macro writes files to disk while your crate compiles,
> creating `<workspace root>/.tasks/<crate>/<task name>.md` for every `task!`.
> That is the point, but it means a build on a read-only source tree fails.
> Development tool, not something to ship. Work in progress, and the least
> finished thing in this workspace.

A crate I am writing to learn procedural macros, and to fix something that was
annoying me.

## The problem

The same loop every time: `Ctrl+F`, search `todo`, read a comment that says
nothing about the size of the work, find a `todo!()` that panics even though
that code would run fine without it. Then the `// TODO:` comments the compiler
never sees, in a different place from the `todo!()`s. Keeping any of it in sync
was manual, so I never did.

## What it does

One macro carries the metadata, decides whether it panics, and writes the task
down.

```rust
task! {
    name:      "Manager Full",
    prio:      TaskPriority::Normal,
    task_type: TaskType::Implement,
    status:    TaskStatus::Todo,
    effort:    TaskEffort::Normal,
    panic:     true,
    desc:      "Decide what the manager does once every worker is saturated.",
}
```

That call writes `.tasks/live_bot/Manager Full.md`:

```markdown
# MANAGER FULL

|              |                  |
| ------------ | ---------------- |
|**Priority**| Normal |
|**Type**| Implement |
|**Status**| Todo |
|**Effort**| Normal |
|**Panic**| true |
|**Source**| [src/runtime/worker/worker.rs](/src/runtime/worker/worker.rs#L151) |
|**Created**| 18-09-2026 15:11 |
|**Updated**| 22-09-2026 10:41 |
---

## Description
Decide what the manager does once every worker is saturated.

> *Don't forget to delete this file once it's done!*
```

Because `panic` is `true`, the call site also expands to
`todo!("See: ./.tasks/live_bot/Manager Full.md")`, so the block has type `!` and
fits any signature, same as a bare `todo!()` except the message points at the
file describing the work.

| `panic` | expands to | meaning |
|---|---|---|
| `true` | `todo!("See: ./.tasks/...")` | not done, this code must not run |
| `false` | nothing | the code runs, the task is tracked but does not block |

`Created` survives a rebuild, `Updated` is the last regeneration, and an
unchanged task is not rewritten at all.

## How it is built

Three crates: `tasks` (facade), `tasks_macros` (the proc macro), `tasks_args`
(shared enums). No `syn`, no `quote`, no `proc-macro2`: the token stream is
parsed by hand against `proc_macro::TokenTree`, and the date maths is a
hand-rolled civil-from-days conversion rather than `chrono`. Writes go through a
PID-suffixed temporary file and an atomic rename, because cargo and
rust-analyzer expand the macro at the same time.

## Known limitations

- **Every error path is a panic.** A macro that cannot parse its input has to
  stop the build, so panicking is the right behaviour, but the messages are not
  always actionable and several of them point at the macro rather than at what
  you typed.
- **The generated markdown is read back by byte offsets, not parsed.**
  Preserving the `Created` timestamp works by splitting the old and the new file
  at known markers and comparing the rest. It is enough to avoid writing a
  parser, and it means any hand edit, or a file truncated by a crash, makes the
  next build fail until the file is deleted and regenerated.
- Two tasks with the same name in the same crate overwrite each other.
- A task marked `Done` is not removed. You delete the file.
- The board renderer that reads the `.md` files back is not in this crate yet.
- The enum name must be written bare, not path-qualified.
- No retry if the atomic rename fails, which an antivirus or a syncing folder
  can cause.
