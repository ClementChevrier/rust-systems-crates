use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::message::*;
use crate::metrics;

/// Severity of a message, from the least to the most severe. Ordered, so
/// `LogLevel::Warn > LogLevel::Info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    /// Detail useful while developing; filtered out by default.
    Debug,
    /// Normal operation. The default minimum level.
    Info,
    /// Something unexpected that the program recovered from.
    Warn,
    /// A failure. Forces a sync to disk under the default [`SyncPolicy`](crate::SyncPolicy).
    Error,
    /// A failure that needs an operator now.
    Critical,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl LogLevel {
    /// Every level, from the least to the most severe.
    pub const ALL: [Self; 5] = [
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
        Self::Critical,
    ];

    pub(super) const fn as_int(self) -> u8 {
        match self {
            Self::Debug => 0,
            Self::Info => 1,
            Self::Warn => 2,
            Self::Error => 3,
            Self::Critical => 4,
        }
    }

    pub(super) fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Critical => "CRITICAL",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|level| value.eq_ignore_ascii_case(level.as_str()))
    }
}

#[derive(Debug)]
pub(crate) struct LogMessage {
    pub(super) level: LogLevel,
    pub(super) ts: SystemTime,
    pub(super) title: &'static str,
    pub(super) body: Vec<u8>,
}

impl LogMessage {
    pub(super) fn len(&self) -> usize {
        let ts_len = TIMESTAMP_LEN + 3; // `[` + ts + `] `
        let level_len = self.level.as_str().len() + 3; // `[` + level + `] `
        // The title is not padded by `write_to`, so its real length is what
        // counts — not TITLE_WIDTH.
        let title_len = self.compact_title().len() + 5; // `[` + title + `] - `
        let body_len = self.body.len() + 1; // body + `\n`

        ts_len + level_len + title_len + body_len
    }

    pub(super) fn write_to<W: Write>(&self, out: &mut W) -> io::Result<()> {
        write!(out, "[")?;
        write_ts(out, self.ts)?;
        write!(out, "] ")?;

        write!(
            out,
            "[{}] [{}] - ",
            self.level.as_str(),
            self.compact_title()
        )?;
        out.write_all(&self.body)?;
        out.write_all(b"\n")
    }

    fn compact_title(&self) -> &str {
        if self.title.len() <= TITLE_WIDTH {
            return self.title;
        }

        let mut end = TITLE_WIDTH;

        while !self.title.is_char_boundary(end) {
            end -= 1;
        }

        &self.title[..end]
    }

    pub(super) fn start() -> Self {
        Self {
            level: LogLevel::Info,
            ts: SystemTime::now(),
            title: "LOGGER",
            body: "Started!".into(),
        }
    }

    pub(super) fn shutdown() -> Self {
        Self {
            level: LogLevel::Info,
            ts: SystemTime::now(),
            title: "LOGGER",
            body: "Received shutdown command!".into(),
        }
    }

    pub(super) fn metrics() -> Self {
        let metrics = metrics::collect();
        Self {
            level: if metrics.is_degraded() {
                LogLevel::Warn
            } else {
                LogLevel::Info
            },
            ts: SystemTime::now(),
            title: "LOGGER",
            body: format!("{metrics}").into_bytes(),
        }
    }

    pub(super) fn dropped(count: u64) -> Self {
        Self {
            level: LogLevel::Warn,
            ts: SystemTime::now(),
            title: "LOGGER",
            body: format!("dropped {count} messages: the channel was full").into_bytes(),
        }
    }

    pub(super) fn senders_dead() -> Self {
        Self {
            level: LogLevel::Warn,
            ts: SystemTime::now(),
            title: "LOGGER",
            body: "All senders are disconnected. Closing Logger!".into(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum LogCommand {
    Message(LogMessage),
    Shutdown,
}

fn write_ts<W: Write>(out: &mut W, ts: SystemTime) -> io::Result<()> {
    let dur = ts.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (y, mo, d, h, m, s) = unix_secs_to_utc(dur.as_secs());
    let ns = dur.subsec_nanos();

    write!(out, "{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}.{ns:09}")
}

pub(crate) fn unix_secs_to_utc(secs: u64) -> (u32, u8, u8, u8, u8, u8) {
    let tod = secs % 86_400;
    let days = secs / 86_400;

    let hh = (tod / 3_600) as u8;
    let mm = ((tod % 3_600) / 60) as u8;
    let ss = (tod % 60) as u8;

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

    (y as u32, mo, d, hh, mm, ss)
}

#[cfg(test)]
#[path = "tests/message.rs"]
mod message_test;
