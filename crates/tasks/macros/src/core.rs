// A proc macro reports bad input by panicking: the compiler turns the panic
// into an error at the call site.
#![allow(clippy::panic)]

use proc_macro::TokenTree;
use std::{
    fmt::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use tasks_args::*;

#[derive(Debug)]
pub(super) struct Task {
    // Required
    name: String,
    prio: TaskPriority,
    task_type: TaskType,
    status: TaskStatus,
    effort: TaskEffort,
    panic: bool,

    // Optional
    desc: Option<String>,
    blocked_by: Option<String>,

    // Filled in by the macro
    created_at: SystemTime,
    loc: Location,
}

fn generate_row(buf: &mut String, name: &str, value: &str) {
    let _ = writeln!(buf, "|**{name}**| {value} |");
}

impl Task {
    pub(super) fn to_md(&self) -> String {
        let mut out = String::new();
        let path = self.loc.file.replace('\\', "/");

        let _ = write!(out, "# {}\n\n", self.name.to_uppercase());

        // Summary table
        out.push_str("|              |                  |\n");
        out.push_str("| ------------ | ---------------- |\n");
        generate_row(&mut out, "Priority", self.prio.as_str());
        generate_row(&mut out, "Type", self.task_type.as_str());
        generate_row(&mut out, "Status", self.status.as_str());
        generate_row(&mut out, "Effort", self.effort.as_str());
        generate_row(&mut out, "Panic", if self.panic { "true" } else { "false" });
        generate_row(
            &mut out,
            "Source",
            &format!("[{path}](/{path}#L{})", self.loc.line),
        );
        // Both rows get the same instant; on an update, `Created` is then put
        // back from the existing file.
        generate_row(&mut out, "Created", &ts_to_string(self.created_at));
        generate_row(&mut out, "Updated", &ts_to_string(self.created_at));

        out.push_str("---\n");

        if let Some(desc) = &self.desc {
            let _ = write!(out, "\n## Description\n{desc}\n");
        }
        if let Some(blocked) = &self.blocked_by {
            let _ = write!(out, "\n## Blocked by\n{blocked}");
        }

        out.push_str("\n> *Don't forget to delete this file once it's done!*");
        out
    }

    /// `<crate>/<name>.md`, relative to the task folder.
    pub(super) fn get_path(&self) -> PathBuf {
        let mut path = PathBuf::new().join(&self.loc.krate).join(&self.name);
        path.add_extension("md");
        path
    }

    pub(super) fn should_panic(&self) -> bool {
        self.panic
    }
}

/// The closest ancestor of the calling crate whose `Cargo.toml` declares a
/// `[workspace]`, or the crate itself.
pub(super) fn workspace_root() -> PathBuf {
    let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        return PathBuf::from(".");
    };
    let manifest_dir = PathBuf::from(manifest_dir);

    if let Some(root) = manifest_dir.ancestors().find(|dir| is_workspace_root(dir)) {
        return root.to_path_buf();
    }

    manifest_dir
}

fn is_workspace_root(dir: &Path) -> bool {
    let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
        return false;
    };

    manifest.lines().any(|line| line.trim() == "[workspace]")
}

/// Where the `task!` call sits.
#[derive(Debug)]
struct Location {
    krate: String,
    file: String,
    line: usize,
}
impl Location {
    fn call_site() -> Self {
        let span = proc_macro::Span::call_site();
        Location {
            krate: std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "UNKNOWN_CRATE".to_string()),
            file: span
                .local_file()
                .map(|file| file.display().to_string())
                .unwrap_or(span.file()),
            line: span.line(),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct TaskBuilder {
    name: Option<String>,
    prio: Option<TaskPriority>,
    task_type: Option<TaskType>,
    status: Option<TaskStatus>,
    effort: Option<TaskEffort>,
    panic: Option<bool>,

    desc: Option<String>,
    blocked_by: Option<String>,
}

impl TaskBuilder {
    pub(super) fn build(self) -> Task {
        Task {
            name: validate_name(self.name.expect("missing field `name`")),
            prio: self.prio.expect("missing field `prio`"),
            task_type: self.task_type.expect("missing field `task_type`"),
            status: self.status.expect("missing field `status`"),
            effort: self.effort.expect("missing field `effort`"),
            panic: self.panic.expect("missing field `panic`"),

            desc: self.desc,
            blocked_by: self.blocked_by,

            loc: Location::call_site(),
            created_at: SystemTime::now(),
        }
    }
}

/// The name becomes a file name, so it must stay a single path component.
fn validate_name(name: String) -> String {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        panic!("invalid task name `{name}`: it must be a file name, without `/` or `\\`");
    }
    name
}

/// Collects the tokens of one `field: value` row, up to the next comma.
pub(super) fn parse_attribute<I: Iterator<Item = TokenTree>>(input: &mut I) -> Vec<String> {
    let mut tokens = Vec::new();
    for token in input.by_ref() {
        match token {
            TokenTree::Punct(punct) if punct.as_char() == ',' => break,
            token => tokens.push(token.to_string()),
        }
    }
    tokens
}

pub(super) fn process_row(input: &[String], task: &mut TaskBuilder) {
    let mut input_iter = input.iter();

    // A row reads `field_name: value`.
    let Some(attrib_name) = input_iter.next() else {
        panic!("expected a field name")
    };

    match input_iter.next().map(String::as_str) {
        Some(":") => (),
        _ => panic!("expected `:` after `{attrib_name}`"),
    }

    match attrib_name.as_str() {
        "name" => task.name = Some(build_literal(&mut input_iter)),
        "prio" => task.prio = Some(build_enum::<TaskPriority>(&mut input_iter)),
        "task_type" => task.task_type = Some(build_enum::<TaskType>(&mut input_iter)),
        "status" => task.status = Some(build_enum::<TaskStatus>(&mut input_iter)),
        "effort" => task.effort = Some(build_enum::<TaskEffort>(&mut input_iter)),
        "panic" => task.panic = Some(build_bool(&mut input_iter)),

        "desc" => task.desc = Some(build_literal(&mut input_iter)),
        "blocked_by" => task.blocked_by = Some(build_literal(&mut input_iter)),

        _ => panic!(
            "unknown field `{attrib_name}`; expected one of `name`, `prio`, `task_type`, \
             `status`, `effort`, `panic`, `desc`, `blocked_by`"
        ),
    }
}

/// Parses `EnumName::Variant`.
fn build_enum<A: TaskArgs>(input: &mut std::slice::Iter<'_, String>) -> A {
    let enum_name = A::name();
    match input.next() {
        Some(name) if name.eq_ignore_ascii_case(enum_name) => {}
        Some(name) => panic!("expected `{enum_name}`, found `{name}`"),
        None => panic!("expected `{enum_name}`, found nothing"),
    }

    // `::` arrives as two `:` tokens.
    for _ in 0..2 {
        match input.next() {
            Some(token) if token == ":" => (),
            Some(token) => panic!("expected `::` after `{enum_name}`, found `{token}`"),
            None => panic!("expected `::` after `{enum_name}`, found nothing"),
        }
    }

    let value = input
        .next()
        .unwrap_or_else(|| panic!("expected a `{enum_name}` variant, found nothing"));

    if let Some(extra) = input.next() {
        panic!("unexpected `{extra}` after `{enum_name}::{value}`");
    }

    A::from_str(value)
}

const STRING_DELIMITER: char = '"';
fn build_literal(input: &mut std::slice::Iter<'_, String>) -> String {
    let Some(raw) = input.next() else {
        panic!("expected a string literal, found nothing")
    };

    if let Some(extra) = input.next() {
        panic!("unexpected `{extra}` after the string literal");
    }

    let Some(literal) = raw
        .strip_prefix(STRING_DELIMITER)
        .and_then(|v| v.strip_suffix(STRING_DELIMITER))
    else {
        panic!("expected a string literal, found `{raw}`")
    };

    literal.to_string()
}

fn build_bool(input: &mut std::slice::Iter<'_, String>) -> bool {
    let Some(raw) = input.next() else {
        panic!("expected `true` or `false`, found nothing")
    };

    if let Some(extra) = input.next() {
        panic!("unexpected `{extra}` after `{raw}`");
    }

    match raw.as_str() {
        "true" => true,
        "false" => false,
        _ => panic!("expected `true` or `false`, found `{raw}`"),
    }
}

/// `DD-MM-YYYY HH:MM`, in UTC.
fn ts_to_string(ts: SystemTime) -> String {
    let dur = ts.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (y, mo, d, h, m) = unix_secs_to_utc(dur.as_secs());

    format!("{d:02}-{mo:02}-{y:04} {h:02}:{m:02}")
}

/// Civil date and time from Unix seconds (Howard Hinnant's `civil_from_days`).
fn unix_secs_to_utc(secs: u64) -> (u32, u8, u8, u8, u8) {
    let tod = secs % 86_400;
    let days = secs / 86_400;

    let hh = (tod / 3_600) as u8;
    let mm = ((tod % 3_600) / 60) as u8;

    let z = days as i64 + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = if mo <= 2 { y + 1 } else { y };

    (y as u32, mo, d, hh, mm)
}
