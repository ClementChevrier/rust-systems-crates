//! The task metadata enums, shared by the `task!` macro, which parses them from
//! tokens, and by the `tasks` facade, which re-exports them.

// `from_str` is called at macro expansion time, where an invalid variant can
// only be reported by panicking.
#![allow(clippy::panic)]

/// An enum that can appear in a `task!` field as `EnumName::Variant`.
pub trait TaskArgs {
    /// The enum's name, as written in the macro call.
    fn name() -> &'static str;
    /// Parses a variant name, case-insensitively. Panics on an unknown one.
    fn from_str(input: &str) -> Self;
    /// The variant name, as written in the generated markdown.
    fn as_str(&self) -> &'static str;
}

/// How soon the task should be done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPriority {
    /// Blocks something now.
    Urgent,
    /// Next in line.
    High,
    /// The default.
    Normal,
    /// When there is time.
    Low,
    /// Nice to have.
    Bonus,
}

impl TaskArgs for TaskPriority {
    fn name() -> &'static str {
        "TaskPriority"
    }
    fn from_str(input: &str) -> TaskPriority {
        match input.to_lowercase().as_str() {
            "urgent" => TaskPriority::Urgent,
            "high" => TaskPriority::High,
            "normal" => TaskPriority::Normal,
            "low" => TaskPriority::Low,
            "bonus" => TaskPriority::Bonus,
            _ => panic!("invalid priority `{input}`"),
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Self::Urgent => "Urgent",
            Self::High => "High",
            Self::Normal => "Normal",
            Self::Low => "Low",
            Self::Bonus => "Bonus",
        }
    }
}

/// What kind of work the task is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    /// Reshape existing code.
    Refactor,
    /// Write something that does not exist yet.
    Implement,
    /// Anything else.
    Others,
}

impl TaskArgs for TaskType {
    fn name() -> &'static str {
        "TaskType"
    }
    fn from_str(input: &str) -> Self {
        match input.to_lowercase().as_str() {
            "refactor" => TaskType::Refactor,
            "implement" => TaskType::Implement,
            "others" => TaskType::Others,
            _ => panic!("invalid task_type `{input}`"),
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Self::Refactor => "Refactor",
            Self::Implement => "Implement",
            Self::Others => "Others",
        }
    }
}

/// Where the task stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// Not started.
    Todo,
    /// In progress.
    Doing,
    /// Waiting on something else; say what in `blocked_by`.
    Blocked,
    /// Finished. The file is not deleted automatically.
    Done,
}

impl TaskArgs for TaskStatus {
    fn name() -> &'static str {
        "TaskStatus"
    }
    fn from_str(input: &str) -> Self {
        match input.to_lowercase().as_str() {
            "todo" => TaskStatus::Todo,
            "doing" => TaskStatus::Doing,
            "blocked" => TaskStatus::Blocked,
            "done" => TaskStatus::Done,
            _ => panic!("invalid status `{input}`"),
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Self::Todo => "Todo",
            Self::Doing => "Doing",
            Self::Blocked => "Blocked",
            Self::Done => "Done",
        }
    }
}

/// Rough size of the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEffort {
    /// Minutes to an hour.
    Quick,
    /// About a day.
    Normal,
    /// Several days.
    Long,
}

impl TaskArgs for TaskEffort {
    fn name() -> &'static str {
        "TaskEffort"
    }
    fn from_str(input: &str) -> Self {
        match input.to_lowercase().as_str() {
            "quick" => TaskEffort::Quick,
            "normal" => TaskEffort::Normal,
            "long" => TaskEffort::Long,
            _ => panic!("invalid effort `{input}`"),
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Self::Quick => "Quick",
            Self::Normal => "Normal",
            Self::Long => "Long",
        }
    }
}
