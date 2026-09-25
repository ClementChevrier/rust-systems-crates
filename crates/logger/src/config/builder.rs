use std::time::Duration;

use crate::{file_manager::SyncPolicy, message::LogLevel};

pub(crate) const DEFAULT_FOLDER_NAME: &str = ".logs";
pub(crate) const DEFAULT_BASE_NAME: &str = "log";
pub(crate) const DEFAULT_INCLUDE_DATE: bool = true;
pub(crate) const DEFAULT_MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
pub(crate) const DEFAULT_THREAD_NAME: &str = "Thread-Logger";
// A typical line is ~130 bytes (29-byte timestamp, level, title of up to 15
// bytes, a ~70-byte body), so 64 KiB holds ~500 lines: a good trade between
// memory and amortising the `write_all` syscall, which stops paying off past
// that size.
pub(crate) const DEFAULT_BUFFER_LEN: u64 = 64 * 1024;
pub(crate) const DEFAULT_MIN_LEVEL: LogLevel = LogLevel::Info;
pub(crate) const DEFAULT_SYNC_POLICY: SyncPolicy = SyncPolicy::OnLevel(LogLevel::Error);

pub(crate) const DEFAULT_SLEEP_MAIN_LOOP: Duration = Duration::from_millis(10);
pub(crate) const DEFAULT_POLL_LEVEL_INTERVAL: Duration = Duration::from_secs(60);
