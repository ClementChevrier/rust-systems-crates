use std::{
    fmt::Display,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::metrics;

use super::message::{LogLevel, LogMessage, unix_secs_to_utc};

/// The first failure is always reported, then one in N. A stalled disk must
/// not turn stderr into a second log, growing faster than the one we failed to
/// write.
const FAILURE_REPORT_EVERY: u64 = 32;

/// When the logger forces written data from the OS cache to the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPolicy {
    /// fsync on rotation and shutdown only. Fastest; a crash loses whatever the
    /// OS had not flushed — potentially a whole file's worth of pages.
    OnRotation,
    /// fsync once a message at this level or above has hit the file. Free in
    /// steady state, durable exactly where it matters.
    OnLevel(LogLevel),
}

struct Date {
    year: u32,
    month: u8,
    day: u8,
}
impl Date {
    const SECS_PER_DAY: u64 = 86_400;

    fn today_as_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn next_day() -> u64 {
        (Self::today_as_secs() / Self::SECS_PER_DAY + 1) * Self::SECS_PER_DAY
    }

    fn today() -> Self {
        let (y, mo, d, _, _, _) = unix_secs_to_utc(Self::today_as_secs());
        Self {
            year: y,
            month: mo,
            day: d,
        }
    }
}
impl PartialEq for Date {
    fn eq(&self, other: &Self) -> bool {
        (self.year, self.month, self.day) == (other.year, other.month, other.day)
    }
}
impl Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

struct NameGenerator {
    include_date: bool,
    current_date: Date,
    number: u32,
}
impl NameGenerator {
    fn new(include_date: bool, number: u32) -> Self {
        Self {
            include_date,
            current_date: Date::today(),
            number,
        }
    }

    /// Name of the current file, without advancing the counter (used to resume
    /// at startup).
    fn get_name_file(&self, base_name: &str) -> PathBuf {
        let name = format!("{base_name}_{}.log", self.number);
        match self.include_date {
            true => PathBuf::from(self.current_date.to_string()).join(name),
            false => PathBuf::from(name),
        }
    }

    /// Name of the next file: number + 1, or back to 0 if the day changed.
    fn next_file(&mut self, base_name: &str) -> PathBuf {
        if self.current_date == Date::today() {
            self.number += 1;
        } else {
            self.current_date = Date::today();
            self.number = 0;
        }

        self.get_name_file(base_name)
    }
}

struct LogFile {
    path: PathBuf,
    inner: fs::File,
    len: u64,

    failure_streak: u64,
}

impl LogFile {
    fn open(name: impl Into<PathBuf>) -> io::Result<Self> {
        let path = name.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let len = file.metadata()?.len();

        Ok(Self {
            path,
            inner: file,
            len,
            failure_streak: 0,
        })
    }

    /// The payload is never echoed: on a full disk stderr is usually another
    /// file on the same volume, and dumping the buffer on every failed flush
    /// fills it faster than the log we could not write.
    fn note_failure(&mut self, written: usize, total: usize, source: Option<&io::Error>) {
        self.failure_streak = self.failure_streak.saturating_add(1);
        metrics::set_write_streak(self.failure_streak);
        let total_failures = metrics::record_write_failure();

        if self.failure_streak == 1 || total_failures.is_multiple_of(FAILURE_REPORT_EVERY) {
            match source {
                Some(e) => eprintln!(
                    "[LOGGER] write to {} failed after {written}/{total} bytes ({} consecutive): {e}",
                    self.path.display(),
                    self.failure_streak
                ),
                None => eprintln!(
                    "[LOGGER] write to {} accepted 0 of {total} bytes ({} consecutive)",
                    self.path.display(),
                    self.failure_streak
                ),
            }
        }
    }

    /// Like `write_all`, but keeps the byte count on a partial write and never
    /// fails the caller: a logger must not take down the program it observes.
    fn write(&mut self, buf: &[u8]) {
        if buf.is_empty() {
            return;
        }

        let mut written = 0;
        while written < buf.len() {
            match self.inner.write(&buf[written..]) {
                // The slice is never empty here, so the file accepted nothing.
                // Unreachable for a regular file — and the break is mandatory:
                // without it `written` never advances and the loop spins.
                Ok(0) => {
                    debug_assert!(false, "File::write returned Ok(0) on a non-empty slice");
                    self.note_failure(written, buf.len(), None);
                    break;
                }
                Ok(n) => written += n,
                // Interrupted means "retry", never "data lost".
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    self.note_failure(written, buf.len(), Some(&e));
                    break;
                }
            }
        }

        if written == buf.len() && self.failure_streak != 0 {
            self.failure_streak = 0;
            metrics::set_write_streak(0);
        }

        self.len += written as u64;
        metrics::record_written(written as u64, (buf.len() - written) as u64)
    }

    fn write_message(&mut self, msg: LogMessage) {
        let mut scratch = Vec::with_capacity(msg.len());
        msg.write_to(&mut scratch)
            .expect("write_to into a Vec is infallible");
        self.write(&scratch)
    }

    fn sync(&self) {
        if let Err(e) = self.inner.sync_data() {
            let failures = metrics::record_fsync_failure();
            if failures == 1 || failures.is_multiple_of(FAILURE_REPORT_EVERY) {
                eprintln!("[LOGGER] fsync {} failed: {e}", self.path.display());
            }
        }
    }
}

pub(super) struct FileManager {
    // File settings
    folder: PathBuf,
    base_name: String,
    max_size: u64,

    // File state
    file: LogFile,
    name_generator: NameGenerator,
    next_day_boundary: u64,

    // Kernel-to-disk sync
    sync_policy: SyncPolicy,

    // Internal
    buff: Vec<u8>,
    flag_closed: bool,
}

impl FileManager {
    pub(crate) fn new(
        folder: PathBuf,
        base_name: String,
        max_size: u64,
        buf_size: u64,
        include_date: bool,
        sync_policy: SyncPolicy,
    ) -> Result<Self, io::Error> {
        fs::create_dir_all(&folder)?;
        let max_number = Self::find_max_used_number(&folder, &base_name, include_date);
        let name_generator = NameGenerator::new(include_date, max_number);
        let file_name = folder.join(name_generator.get_name_file(&base_name));

        Ok(Self {
            base_name,
            max_size,

            file: LogFile::open(file_name)?,
            name_generator,
            folder,
            next_day_boundary: Date::next_day(),

            sync_policy,

            buff: Vec::with_capacity(buf_size as usize),
            flag_closed: false,
        })
    }

    pub(super) fn write(&mut self, msg: LogMessage) {
        let must_sync = matches!(
            self.sync_policy,
            SyncPolicy::OnLevel(threshold) if msg.level.as_int() >= threshold.as_int()
        );

        let encoded_len = msg.len();
        if encoded_len >= self.buff.capacity() {
            self.write_buff_to_file();
            self.file.write_message(msg);
            if must_sync {
                self.file.sync();
            }
            return;
        } else if self.buff.len() + encoded_len >= self.buff.capacity() {
            self.write_buff_to_file();
        }
        msg.write_to(&mut self.buff)
            .expect("write_to into a Vec is infallible");

        if must_sync {
            self.write_buff_to_file();
            self.file.sync();
        }
    }

    #[cfg(test)]
    pub(super) fn flush(&mut self) {
        self.write_buff_to_file();
    }

    fn write_buff_to_file(&mut self) {
        // Rotate first if this flush would overrun the file.
        if self.file.len + self.buff.len() as u64 > self.max_size {
            self.get_next_file();
        }

        self.file.write(&self.buff);
        self.buff.clear();
        metrics::set_file_bytes(self.file.len);

        // A new UTC day starts a new file, in a new folder when dated.
        if self.name_generator.include_date && (Date::today_as_secs() >= self.next_day_boundary) {
            self.get_next_file();
            self.next_day_boundary = Date::next_day();
        }
    }

    fn get_next_file(&mut self) {
        let new_path = self
            .folder
            .join(self.name_generator.next_file(&self.base_name));
        match LogFile::open(&new_path) {
            Ok(file) => {
                self.file.write_message(LogMessage::metrics());
                self.file.sync();
                self.file = file;
                metrics::set_file_bytes(self.file.len);
            }
            Err(e) => {
                metrics::record_rotation_failure();
                eprintln!("[LOGGER] cannot open {}: {e}", new_path.display());
            }
        }
    }

    fn find_max_used_number(folder: &Path, base_name: &str, include_date: bool) -> u32 {
        let dir = match include_date {
            true => folder.join(Date::today().to_string()),
            false => folder.to_path_buf(),
        };
        let Ok(entries) = fs::read_dir(&dir) else {
            return 0;
        };

        let prefix = format!("{base_name}_");
        let mut max_number: u32 = 0;
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };

            let number = parse_file_number(&name, &prefix);

            if let Some(number) = number
                && number > max_number
            {
                max_number = number;
            }
        }
        max_number
    }

    pub(super) fn shutdown(&mut self) {
        if self.flag_closed {
            return;
        }
        self.flag_closed = true;

        self.write_buff_to_file();
        self.file.sync();
    }
}

/// The `<n>` of a `<prefix><n>.log` file name, or `None` for any other name.
///
/// Only digits may sit between the prefix and the extension, so a dated name
/// such as `log_2026-07-11_3.log` is not mistaken for file number 3.
fn parse_file_number(file_name: &str, prefix: &str) -> Option<u32> {
    file_name
        .strip_prefix(prefix)?
        .strip_suffix(".log")?
        .parse::<u32>()
        .ok()
}

impl Drop for FileManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(all(test, not(miri)))]
#[path = "tests/file_manager.rs"]
mod file_manager_tests;
