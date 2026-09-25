#![allow(clippy::unwrap_used, clippy::panic)]

//! resolve_min_level touches neither MIN_LEVEL nor the singleton, so these
//! tests run in parallel, each with its own control file.

use super::resolve_min_level;
use crate::LogLevel;
use crate::config::builder::DEFAULT_MIN_LEVEL;

use std::fs;
use std::path::PathBuf;

fn temp_file(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("logger_level_{}_{tag}.ctl", std::process::id()))
}

#[test]
fn explicit_level_wins_over_the_file_and_is_persisted() {
    let path = temp_file("explicit");
    fs::write(&path, "error").expect("seed control file");

    let level = resolve_min_level(&path, Some(LogLevel::Debug));

    assert_eq!(level.as_int(), LogLevel::Debug.as_int());
    assert_eq!(fs::read_to_string(&path).expect("read"), "DEBUG");
    let _ = fs::remove_file(&path);
}

#[test]
fn file_level_wins_when_no_explicit_level() {
    let path = temp_file("from_file");
    fs::write(&path, "warn").expect("seed control file");

    let level = resolve_min_level(&path, None);

    assert_eq!(level.as_int(), LogLevel::Warn.as_int());
    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "warn",
        "file must not be rewritten"
    );
    let _ = fs::remove_file(&path);
}

#[test]
fn invalid_file_content_falls_back_to_the_default() {
    let path = temp_file("invalid");
    fs::write(&path, "verbose").expect("seed control file");

    assert_eq!(
        resolve_min_level(&path, None).as_int(),
        DEFAULT_MIN_LEVEL.as_int()
    );
    let _ = fs::remove_file(&path);
}

#[test]
fn missing_file_is_seeded_with_the_default() {
    let path = temp_file("missing");
    let _ = fs::remove_file(&path);

    let level = resolve_min_level(&path, None);

    assert_eq!(level.as_int(), DEFAULT_MIN_LEVEL.as_int());
    assert_eq!(
        fs::read_to_string(&path).expect("file must be seeded"),
        DEFAULT_MIN_LEVEL.as_str()
    );
    let _ = fs::remove_file(&path);
}
