#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

fn ts(secs: u64) -> String {
    ts_precise(secs, 0)
}

fn ts_precise(secs: u64, nanos: u32) -> String {
    let mut out = Vec::new();
    let moment = UNIX_EPOCH + std::time::Duration::new(secs, nanos);
    write_ts(&mut out, moment).expect("write_ts");
    String::from_utf8(out).expect("timestamp is ascii")
}

fn message(level: LogLevel, title: &'static str, body: &str) -> LogMessage {
    LogMessage {
        level,
        ts: SystemTime::now(),
        title,
        body: body.into(),
    }
}

fn encoded(msg: &LogMessage) -> Vec<u8> {
    let mut out = Vec::new();
    msg.write_to(&mut out).expect("write_to a Vec cannot fail");
    out
}

fn line(level: LogLevel, title: &'static str, body: &str) -> String {
    String::from_utf8(encoded(&message(level, title, body))).expect("line must be valid utf-8")
}

mod timestamp {
    use super::*;

    #[test]
    fn epoch() {
        assert_eq!(ts(0), "1970-01-01 00:00:00.000000000");
    }

    #[test]
    fn before_epoch_clamps_to_epoch() {
        let mut out = Vec::new();
        let before = UNIX_EPOCH - std::time::Duration::from_secs(10);
        write_ts(&mut out, before).expect("write_ts");
        assert_eq!(
            String::from_utf8(out).expect("ascii"),
            "1970-01-01 00:00:00.000000000"
        );
    }

    #[test]
    fn last_second_of_the_millennium() {
        assert_eq!(ts(946_684_799), "1999-12-31 23:59:59.000000000");
    }

    #[test]
    fn first_second_of_2000() {
        assert_eq!(ts(946_684_800), "2000-01-01 00:00:00.000000000");
    }

    #[test]
    fn leap_day_2024() {
        assert_eq!(ts(1_709_210_096), "2024-02-29 12:34:56.000000000");
    }

    #[test]
    fn leap_century_2000() {
        assert_eq!(ts(951_782_400), "2000-02-29 00:00:00.000000000");
    }

    #[test]
    fn non_leap_century_2100_has_no_february_29() {
        assert_eq!(ts(4_107_456_000), "2100-02-28 00:00:00.000000000");
        assert_eq!(ts(4_107_456_000 + 86_400), "2100-03-01 00:00:00.000000000");
    }

    #[test]
    fn famous_32bit_overflow_timestamp() {
        assert_eq!(ts(2_147_483_647), "2038-01-19 03:14:07.000000000");
    }
}

mod levels {
    use super::*;

    const ALL: [LogLevel; 4] = [
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ];

    #[test]
    fn as_int_is_strictly_increasing_with_severity() {
        for pair in ALL.windows(2) {
            assert!(pair[0].as_int() < pair[1].as_int());
        }
    }

    #[test]
    fn ord_matches_severity() {
        assert!(LogLevel::Debug.as_int() < LogLevel::Info.as_int());
        assert!(LogLevel::Info.as_int() < LogLevel::Warn.as_int());
        assert!(LogLevel::Warn.as_int() < LogLevel::Error.as_int());
    }

    #[test]
    fn from_str_roundtrips_every_level() {
        for level in ALL {
            assert_eq!(
                LogLevel::from_str(level.as_str()).unwrap().as_int(),
                level.as_int()
            );
        }
    }

    #[test]
    fn from_str_is_case_and_whitespace_tolerant() {
        assert_eq!(
            LogLevel::from_str(" DeBuG \n").unwrap().as_int(),
            LogLevel::Debug.as_int()
        );
        assert_eq!(
            LogLevel::from_str("\tERROR").unwrap().as_int(),
            LogLevel::Error.as_int()
        );
    }

    #[test]
    fn from_str_rejects_garbage() {
        for bad in ["", "warning", "inf", "debug info", "0"] {
            assert!(LogLevel::from_str(bad).is_none());
        }
    }
}

mod encoding {
    use super::*;

    #[test]
    fn encoded_len_is_exact_for_every_level_and_title_shape() {
        let titles = [
            "A",
            "EXACTLY_12ch",
            "WAY_TOO_LONG_FOR_THE_COLUMN",
            "tïtré_unicode_très_long",
            "a€€€€",
        ];
        let bodies = ["body with some content", ""];
        for title in titles {
            for body in bodies {
                for level in [
                    LogLevel::Debug,
                    LogLevel::Info,
                    LogLevel::Warn,
                    LogLevel::Error,
                ] {
                    let msg = message(level, title, body);
                    let bytes = encoded(&msg);
                    assert_eq!(bytes.len(), msg.len(), "title: {title}, body: {body:?}");
                }
            }
        }
    }

    #[test]
    fn timestamp_column_is_aligned_across_levels() {
        let reference = line(LogLevel::Debug, "T", "b")
            .find(" [")
            .expect("timestamp bracket");
        for level in [LogLevel::Info, LogLevel::Warn, LogLevel::Error] {
            assert_eq!(
                line(level, "T", "b").find(" ["),
                Some(reference),
                "level: {level:?}"
            );
        }
    }

    #[test]
    fn ascii_title_is_cut_at_exactly_title_width_bytes() {
        let too_long: &'static str = "X".repeat(TITLE_WIDTH + 1).leak();
        let out = line(LogLevel::Info, too_long, "x");
        assert!(
            out.contains(&format!("[{}]", &too_long[..TITLE_WIDTH])),
            "line: {out}"
        );
        assert!(!out.contains(too_long));
    }

    #[test]
    fn full_width_title_is_kept_whole_without_padding() {
        let exact: &'static str = "T".repeat(TITLE_WIDTH).leak();
        let out = line(LogLevel::Info, exact, "x");
        assert!(out.contains(&format!("[{exact}] - ")), "line: {out}");
    }

    #[test]
    fn multibyte_title_is_cut_on_a_char_boundary() {
        // 'a' + n x '€' (3 bytes each): unless TITLE_WIDTH % 3 == 1, the limit
        // falls inside a '€' and the cut must step back to the previous char
        // boundary, keeping valid UTF-8.
        let title: &'static str = format!("a{}", "€".repeat(TITLE_WIDTH)).leak();
        let kept = 1 + 3 * ((TITLE_WIDTH - 1) / 3); // largest 1 + 3k <= TITLE_WIDTH
        let out = line(LogLevel::Info, title, "x");
        assert!(
            out.contains(&format!("[{}]", &title[..kept])),
            "line: {out}"
        );
    }

    #[test]
    fn output_ends_with_newline() {
        let msg = message(LogLevel::Error, "T", "b");
        assert_eq!(encoded(&msg).last(), Some(&b'\n'));
    }
}
