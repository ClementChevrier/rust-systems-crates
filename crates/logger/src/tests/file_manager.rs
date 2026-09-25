#![allow(clippy::unwrap_used, clippy::panic)]

//! Size rotation once had a bug (the number was never incremented) that went
//! unnoticed because no test covered it: here every file-name transition is
//! pinned down. The day change is tested on NameGenerator (hermetic, with a
//! faked date) because FileManager reads the clock.

use super::{Date, FileManager, NameGenerator};
use crate::message::{LogLevel, LogMessage};

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const TINY_MAX_SIZE: u64 = 10;
const NO_ROTATION: u64 = u64::MAX;
const BUFFER_LEN: u64 = 4096;

fn temp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("logger_fm_{}_{tag}", std::process::id()))
}

fn msg(body: &str) -> LogMessage {
    LogMessage {
        level: LogLevel::Info,
        ts: SystemTime::now(),
        title: "TEST",
        body: body.into(),
    }
}

/// Message dont l'encodage fait exactement `target` octets.
fn msg_of_encoded_len(target: usize) -> LogMessage {
    let overhead = msg("").len();
    assert!(target >= overhead, "target too small: min is {overhead}");
    msg(&"x".repeat(target - overhead))
}

fn manager(dir: &Path, max_size: u64, buffer_len: u64) -> FileManager {
    FileManager::new(
        dir.to_path_buf(),
        "log".to_string(),
        max_size,
        buffer_len,
        false,
        crate::SyncPolicy::OnRotation,
    )
    .expect("file manager must be creatable")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = temp_dir(tag);
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn file_name(fm: &FileManager) -> String {
    fm.file
        .path
        .file_name()
        .expect("log file has a name")
        .to_string_lossy()
        .into_owned()
}

fn read(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap_or_default()
}

mod naming {
    use super::*;

    fn generator_on(date: Date, number: u32, include_date: bool) -> NameGenerator {
        NameGenerator {
            include_date,
            current_date: date,
            number,
        }
    }

    #[test]
    fn current_file_formats_with_and_without_date() {
        let date = Date {
            year: 2026,
            month: 7,
            day: 5,
        };

        assert_eq!(
            generator_on(date, 3, true).get_name_file("log"),
            PathBuf::from("2026-07-05\\log_3.log")
        );
        let date = Date {
            year: 2026,
            month: 7,
            day: 5,
        };
        assert_eq!(
            generator_on(date, 3, false).get_name_file("log"),
            PathBuf::from("log_3.log")
        );
    }

    #[test]
    fn current_file_never_advances_the_counter() {
        let generator = NameGenerator::new(false, 5);

        assert_eq!(
            generator.get_name_file("log"),
            generator.get_name_file("log")
        );
        assert_eq!(generator.number, 5);
    }

    #[test]
    fn next_file_increments_within_the_same_day() {
        let mut generator = NameGenerator::new(false, 5);

        assert_eq!(generator.next_file("log"), PathBuf::from("log_6.log"));
        assert_eq!(generator.next_file("log"), PathBuf::from("log_7.log"));
    }

    #[test]
    fn next_file_resets_to_zero_when_the_day_changes() {
        let past = Date {
            year: 2000,
            month: 1,
            day: 1,
        };
        let mut generator = generator_on(past, 7, true);

        let path = generator.next_file("log");

        assert_eq!(generator.number, 0);
        let expected = format!("{}\\log_0.log", Date::today());
        assert_eq!(path, PathBuf::from(expected));
    }

    #[test]
    fn date_display_is_zero_padded() {
        let date = Date {
            year: 2026,
            month: 7,
            day: 5,
        };

        assert_eq!(date.to_string(), "2026-07-05");
    }

    #[test]
    fn next_day_is_a_future_midnight_boundary() {
        let now = Date::today_as_secs();
        let boundary = Date::next_day();

        assert_eq!(boundary % Date::SECS_PER_DAY, 0);
        assert!(boundary > now);
        assert!(boundary - now <= Date::SECS_PER_DAY);
    }
}

mod number_extraction {
    use crate::file_manager::parse_file_number;

    #[test]
    fn valid_names_yield_their_number() {
        assert_eq!(parse_file_number("log_0.log", "log_"), Some(0));
        assert_eq!(parse_file_number("log_42.log", "log_"), Some(42));
        assert_eq!(
            parse_file_number("log_2026-07-11_3.log", "log_2026-07-11_"),
            Some(3)
        );
    }

    #[test]
    fn dated_files_do_not_pollute_the_undated_scan() {
        // Regression: a naive split used to read "2026-07-11_3" as 3.
        assert_eq!(parse_file_number("log_2026-07-11_3.log", "log_"), None);
    }

    #[test]
    fn foreign_names_are_rejected() {
        assert_eq!(parse_file_number("other_5.log", "log_"), None);
        assert_eq!(parse_file_number("log_abc.log", "log_"), None);
        assert_eq!(parse_file_number("log_5.txt", "log_"), None);
        assert_eq!(parse_file_number("log_.log", "log_"), None);
        assert_eq!(parse_file_number("log_-1.log", "log_"), None);
    }
}

mod startup {
    use super::*;

    #[test]
    fn new_creates_the_output_folder_and_first_file() {
        let dir = fresh_dir("creates");

        let fm = manager(&dir, NO_ROTATION, BUFFER_LEN);

        assert_eq!(file_name(&fm), "log_0.log");
        assert!(fm.file.path.exists());
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_resumes_the_highest_numbered_file() {
        let dir = fresh_dir("resume");
        fs::create_dir_all(&dir).expect("create dir");
        fs::write(dir.join("log_7.log"), "old\n").expect("seed file");
        fs::write(dir.join("log_10.log"), "older\n").expect("seed file");

        let fm = manager(&dir, NO_ROTATION, BUFFER_LEN);

        // 10 > 7 numerically (a lexicographic sort would have picked 7).
        assert_eq!(file_name(&fm), "log_10.log");
        assert_eq!(
            fm.file.len,
            "older\n".len() as u64,
            "resume must append, not truncate"
        );
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_ignores_unrelated_entries() {
        let dir = fresh_dir("ignores");
        fs::create_dir_all(&dir).expect("create dir");
        fs::write(dir.join("log_2026-01-01_9.log"), "").expect("seed"); // dated
        fs::write(dir.join("otherbase_7.log"), "").expect("seed"); // other base name
        fs::write(dir.join("log_x.log"), "").expect("seed"); // not a number
        fs::write(dir.join("log_5.txt"), "").expect("seed"); // wrong extension
        fs::create_dir_all(dir.join("log_99.log")).expect("seed"); // a directory with a matching name

        let fm = manager(&dir, NO_ROTATION, BUFFER_LEN);

        assert_eq!(file_name(&fm), "log_0.log");
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dated_startup_resumes_todays_highest_file() {
        // fresh_dir wipes the folder: call it once, before seeding anything.
        let root = fresh_dir("resume_dated");
        let day = root.join(Date::today().to_string());
        fs::create_dir_all(&day).expect("create dir");
        fs::write(day.join("log_2.log"), "").expect("seed");
        fs::write(day.join("log_9.log"), "").expect("seed");

        let fm = FileManager::new(
            root.clone(),
            "log".to_string(),
            NO_ROTATION,
            BUFFER_LEN,
            true,
            crate::SyncPolicy::OnRotation,
        )
        .expect("file manager must be creatable");

        assert_eq!(file_name(&fm), "log_9.log");
        drop(fm);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn new_fails_when_the_folder_path_is_a_file() {
        let dir = fresh_dir("collision");
        fs::create_dir_all(dir.parent().expect("temp parent")).expect("create parent");
        fs::write(&dir, "not a folder").expect("seed blocking file");

        let result = FileManager::new(
            dir.clone(),
            "log".to_string(),
            NO_ROTATION,
            BUFFER_LEN,
            false,
            crate::SyncPolicy::OnRotation,
        );

        assert!(result.is_err());
        let _ = fs::remove_file(&dir);
    }
}

mod rotation {
    use super::*;

    #[test]
    fn size_rotation_moves_to_the_next_number() {
        // Regression test for that bug: rotation goes _0 then _1, never the same file.
        let dir = fresh_dir("rotate");
        let mut fm = manager(&dir, TINY_MAX_SIZE, BUFFER_LEN);
        let first = fm.file.path.clone();

        fm.write(msg("first"));
        fm.flush();

        assert_eq!(file_name(&fm), "log_1.log");
        assert_ne!(
            fm.file.path, first,
            "rotation must not reopen the same file"
        );
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotated_files_form_a_strictly_increasing_sequence() {
        let dir = fresh_dir("sequence");
        let mut fm = manager(&dir, TINY_MAX_SIZE, BUFFER_LEN);

        for i in 0..3 {
            fm.write(msg(&format!("message {i}")));
            fm.flush();
        }

        assert_eq!(file_name(&fm), "log_3.log");
        for i in 0..3 {
            let content = read(&dir, &format!("log_{}.log", i + 1));
            assert!(
                content.contains(&format!("message {i}")),
                "log_{}.log: {content}",
                i + 1
            );
        }
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn closed_file_stops_growing_after_rotation() {
        let dir = fresh_dir("frozen");
        let mut fm = manager(&dir, TINY_MAX_SIZE, BUFFER_LEN);

        fm.write(msg("in file zero"));
        fm.flush();
        let frozen_len = read(&dir, "log_0.log").len();

        fm.write(msg("in file one"));
        fm.flush();

        assert_eq!(read(&dir, "log_0.log").len(), frozen_len);
        assert!(read(&dir, "log_1.log").contains("in file zero"));
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_rotation_below_max_size() {
        let dir = fresh_dir("norotate");
        let mut fm = manager(&dir, NO_ROTATION, BUFFER_LEN);

        for i in 0..50 {
            fm.write(msg(&format!("message {i}")));
        }

        assert_eq!(file_name(&fm), "log_0.log");
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }
}

mod buffering {
    use super::*;

    #[test]
    fn small_messages_stay_buffered_until_flush() {
        let dir = fresh_dir("buffered");
        let mut fm = manager(&dir, NO_ROTATION, BUFFER_LEN);

        fm.write(msg("hello"));
        assert_eq!(read(&dir, "log_0.log").len(), 0, "buffered before flush");

        fm.flush();
        assert!(read(&dir, "log_0.log").contains("hello"));
        assert!(fm.buff.is_empty());
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn message_of_exactly_buffer_capacity_bypasses_the_buffer() {
        let dir = fresh_dir("exact_cap");
        let mut fm = manager(&dir, NO_ROTATION, 128);

        fm.write(msg_of_encoded_len(128));

        assert_eq!(
            read(&dir, "log_0.log").len(),
            128,
            "must hit disk immediately"
        );
        assert!(fm.buff.is_empty());
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn message_one_byte_below_capacity_is_buffered() {
        let dir = fresh_dir("below_cap");
        let mut fm = manager(&dir, NO_ROTATION, 128);

        fm.write(msg_of_encoded_len(127));

        assert_eq!(fm.buff.len(), 127);
        assert_eq!(read(&dir, "log_0.log").len(), 0);
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn buffer_flushes_before_overflowing() {
        let dir = fresh_dir("preflush");
        let mut fm = manager(&dir, NO_ROTATION, 128);

        fm.write(msg_of_encoded_len(100));
        fm.write(msg_of_encoded_len(100));

        // The second one no longer fit: the first went to disk first.
        assert_eq!(read(&dir, "log_0.log").len(), 100);
        assert_eq!(fm.buff.len(), 100);
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn buffer_never_reallocates() {
        let dir = fresh_dir("norealloc");
        let mut fm = manager(&dir, NO_ROTATION, BUFFER_LEN);
        let capacity_before = fm.buff.capacity();

        for i in 0..100 {
            fm.write(msg(&format!("message number {i}")));
        }

        assert_eq!(fm.buff.capacity(), capacity_before);
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }
}

mod drops_and_shutdown {
    use super::*;

    #[test]
    fn shutdown_flushes_pending_and_is_idempotent() {
        let dir = fresh_dir("shutdown");
        let mut fm = manager(&dir, NO_ROTATION, BUFFER_LEN);

        fm.write(msg("pending"));
        fm.shutdown();
        let after_first = read(&dir, "log_0.log");
        fm.shutdown(); // idempotent: flag_closed short-circuits, no rewrite, no panic

        let content = read(&dir, "log_0.log");
        assert!(content.contains("pending"));
        assert_eq!(content, after_first, "the second shutdown must be a no-op");
        // The shutdown marker is written by the worker, not by FileManager.
        drop(fm);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn drop_flushes_pending_data() {
        let dir = fresh_dir("drop_impl");
        let mut fm = manager(&dir, NO_ROTATION, BUFFER_LEN);

        fm.write(msg("never explicitly flushed"));
        drop(fm);

        let content = read(&dir, "log_0.log");
        assert!(content.contains("never explicitly flushed"));
        let _ = fs::remove_dir_all(&dir);
    }
}
