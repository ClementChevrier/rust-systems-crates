//! Turns a `todo!()` into a task the repository can show.
//!
//! `task!` records a task where the work is actually needed. It always writes it
//! as markdown under `.tasks/<crate>/<name>.md` at the workspace root, carrying
//! priority, type, status, effort, creation date and a link back to the source
//! line.
//!
//! What it *expands to* is decided by `panic`:
//!
//! | `panic` | Expansion                  | Use it when                                                 |
//! |---------|----------------------------|-------------------------------------------------------------|
//! | `true`  | `todo!("See: ./.tasks/…")` | The work is not done. The code must not run.                |
//! | `false` | nothing                    | The code works; the task only records something to improve. |
//!
//! `panic: true` has type `!`, so the call site type-checks anywhere: that is
//! what lets an unwritten function body be just a `task!`. `panic: false`
//! expands to an empty block of type `()`, so it is only valid in **statement
//! position**: as the trailing expression of a function returning anything other
//! than `()`, it will not compile.
//!
//! ```ignore
//! // The block must take the type the signature demands, hence `panic: true`.
//! fn health_score(&self) -> f64 {
//!     task! {
//!         name: "pool-health-score",
//!         prio: TaskPriority::Normal,
//!         task_type: TaskType::Implement,
//!         status: TaskStatus::Todo,
//!         effort: TaskEffort::Normal,
//!         panic: true,
//!         desc: "Aggregate per-worker latency into a single score.",
//!     }
//! }
//!
//! // Working code, with a note attached to it.
//! fn rotate(&mut self) {
//!     task! {
//!         name: "rotation-backoff",
//!         prio: TaskPriority::High,
//!         task_type: TaskType::Refactor,
//!         status: TaskStatus::Todo,
//!         effort: TaskEffort::Quick,
//!         panic: false,
//!         desc: "Back off instead of retrying the open on every flush.",
//!     }
//!     self.open_next();
//! }
//! ```
//!
//! Required: `name`, `prio`, `task_type`, `status`, `effort`, `panic`.
//! Optional: `desc`, `blocked_by`.
//!
//! # Writing to disk at compile time
//!
//! The macro touches the filesystem while the crate compiles. That is what turns
//! a task into a tracked file instead of a comment nobody reads. It is a
//! development tool and is not meant to run inside a shipped binary.
//!
//! # Known limitations
//!
//! - Two tasks sharing a name inside the same crate overwrite each other: the
//!   path is built from the crate name and the task name only.
//! - Completed tasks are never removed from `.tasks/`; a task set to `Done` stays
//!   on disk until deleted by hand.
//! - Rendering `.tasks/` as a board, and parsing the markdown back, are out of
//!   scope: the files are plain markdown for a human or an external tool to read.

pub use tasks_args::*;
pub use tasks_macros::task;
