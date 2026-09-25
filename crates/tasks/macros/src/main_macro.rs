// A proc macro reports bad input by panicking: the compiler turns the panic
// into an error at the call site.
#![allow(clippy::panic)]

use proc_macro::*;
use std::{ops::Range, path::Path};

use super::core::*;

pub(crate) const DEFAULT_FOLDER_NAME: &str = ".tasks";

pub(crate) fn task(input: TokenStream) -> TokenStream {
    let mut body = type_hints(collect_enum_paths(&input));

    let task = build_task(input);
    publish_task(&task);

    let tail = if task.should_panic() {
        let relative = task.get_path().display().to_string().replace('\\', "/");
        format!(
            "todo!({:?})",
            format!("See: ./{DEFAULT_FOLDER_NAME}/{relative}")
        )
    } else {
        String::new()
    };

    body.extend(tail.parse::<TokenStream>().unwrap_or_default());
    TokenStream::from(TokenTree::Group(Group::new(Delimiter::Brace, body)))
}

// Next block is done by IA in order to have autocomplete in the IDE!
fn collect_enum_paths(input: &TokenStream) -> Vec<Vec<TokenTree>> {
    let tokens: Vec<TokenTree> = input.clone().into_iter().collect();
    let mut paths = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        let is_path_head = matches!(tokens.get(i), Some(TokenTree::Ident(_)))
            && is_colon(tokens.get(i + 1))
            && is_colon(tokens.get(i + 2));

        if !is_path_head {
            i += 1;
            continue;
        }

        let mut path = tokens[i..i + 3].to_vec();
        match tokens.get(i + 3) {
            Some(variant @ TokenTree::Ident(_)) => {
                path.push(variant.clone());
                i += 4;
            }
            _ => {
                let anchor = tokens[i + 2].span();
                path.push(TokenTree::Ident(Ident::new("__incomplete", anchor)));
                i += 3;
            }
        }
        paths.push(path);
    }
    paths
}

fn is_colon(token: Option<&TokenTree>) -> bool {
    matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == ':')
}

fn type_hints(paths: Vec<Vec<TokenTree>>) -> TokenStream {
    if paths.is_empty() {
        return TokenStream::new();
    }

    let mut inner = TokenStream::new();
    for path in paths {
        inner.extend(path);
        inner.extend([TokenTree::Punct(Punct::new(';', Spacing::Alone))]);
    }

    let mut out: TokenStream = "#[allow(path_statements, unused)] const _: fn() = ||"
        .parse()
        .unwrap_or_default();
    out.extend([
        TokenTree::Group(Group::new(Delimiter::Brace, inner)),
        TokenTree::Punct(Punct::new(';', Spacing::Alone)),
    ]);
    out
}
// end of IA block

fn build_task(input: TokenStream) -> Task {
    let mut iter_input = input.into_iter().peekable();
    let mut task = TaskBuilder::default();
    while let Some(token) = iter_input.peek() {
        match token {
            TokenTree::Ident(_) => {
                let row = parse_attribute(&mut iter_input);
                process_row(&row, &mut task)
            }
            TokenTree::Group(group) => {
                panic!("unexpected group `{group}`: expected `field: value`")
            }
            other => panic!("unexpected `{other}`: expected a field name"),
        }
    }

    task.build()
}

/// Writes the task file, or updates it if its content changed.
pub(super) fn publish_task(task: &Task) {
    let path = workspace_root()
        .join(DEFAULT_FOLDER_NAME)
        .join(task.get_path());

    match std::fs::read(&path) {
        Err(_) => write_task(&path, task.to_md().as_bytes()),
        Ok(old_task) => update_task(&path, task, &old_task),
    };
}

fn update_task(path: &Path, task: &Task, old_task: &[u8]) {
    let mut new_task = task.to_md();

    if task_is_unchanged(old_task, new_task.as_bytes()) {
        return;
    }

    let old_task = std::str::from_utf8(old_task)
        .unwrap_or_else(|_| panic!("task file {} is not valid UTF-8", path.display()));
    preserve_created_timestamp(old_task, &mut new_task);
    write_task(path, new_task.as_bytes());
}

/// The task already exists: keep its `Created` timestamp and let only
/// `Updated` move.
///
/// Like [`task_is_unchanged`], this relies on the markers of the generated
/// markdown rather than on a parser. Both ranges start right after an ASCII
/// marker and end right before one, so they fall on char boundaries.
fn preserve_created_timestamp(old_task: &str, new_task: &mut String) {
    let old_created = range_between(old_task.as_bytes(), b"Created", b"Updated");
    let new_created = range_between(new_task.as_bytes(), b"Created", b"Updated");

    new_task.replace_range(new_created, &old_task[old_created]);
}

/// Two renderings of the same task differ only in their timestamps, so compare
/// everything before the `Created` row and everything from the `---` separator
/// on. This avoids writing a parser for the generated markdown.
fn task_is_unchanged(old_task: &[u8], new_task: &[u8]) -> bool {
    let timestamp_start = find_position(new_task, b"Created");
    let separator_start = find_position(&new_task[timestamp_start..], b"---") + timestamp_start;

    let same_header = old_task.get(..timestamp_start) == Some(&new_task[..timestamp_start]);
    let same_tail = old_task.get(separator_start..) == Some(&new_task[separator_start..]);

    same_header && same_tail
}

fn write_task(path: &Path, task: &[u8]) {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .unwrap_or_else(|e| panic!("cannot create {}: {e}", parent.display()));

    // cargo and the IDE can expand the macro at the same time: a PID-suffixed
    // temporary file plus an atomic rename keeps them from writing into the
    // same file.
    let temp_file = parent.join(format!(
        "{}_{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("task"),
        std::process::id(),
    ));

    std::fs::write(&temp_file, task)
        .unwrap_or_else(|e| panic!("cannot write {}: {e}", temp_file.display()));
    std::fs::rename(&temp_file, path).unwrap_or_else(|rename_err| {
        let _ = std::fs::remove_file(&temp_file);
        panic!(
            "cannot move the task file into {}: {rename_err}",
            path.display()
        )
    })
}

fn find_position(origin: &[u8], needle: &[u8]) -> usize {
    origin
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap_or_else(|| {
            panic!(
                "invalid task file: `{}` not found; delete the file to regenerate it",
                String::from_utf8_lossy(needle)
            )
        })
}

fn range_between(origin: &[u8], first: &[u8], second: &[u8]) -> Range<usize> {
    let start = find_position(origin, first) + first.len();
    let end = find_position(origin, second);
    start..end
}
