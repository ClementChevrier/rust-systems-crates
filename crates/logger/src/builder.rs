use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use channel::mpsc;

use super::{
    LogLevel, Logger,
    config::{
        builder::*,
        logger::CHANNEL_LEN,
        state::{logger, min_level},
    },
    error::Error,
    file_manager::{FileManager, SyncPolicy},
    message::LogCommand,
    worker::LogWorker,
};

/// Configures and starts the [`Logger`]. Every setting has a default.
pub struct LoggerBuilder {
    // Destination
    folder_name: PathBuf,
    base_file_name: String,
    include_date: bool,
    max_file_size: u64,

    // Buffering
    buffer_len: u64,
    main_loop_sleep: Duration,

    // Kernel-to-disk sync
    sync_policy: SyncPolicy,

    // Runtime level control
    min_level: Option<LogLevel>,
    runtime_level_file: Option<PathBuf>,
    poll_interval: Option<Duration>,

    // Logger-thread startup hook
    run_thread_init: Box<dyn FnOnce() + Send>,
}

impl Default for LoggerBuilder {
    fn default() -> Self {
        Self {
            folder_name: PathBuf::from(".").join(DEFAULT_FOLDER_NAME),
            base_file_name: DEFAULT_BASE_NAME.to_string(),
            include_date: DEFAULT_INCLUDE_DATE,
            max_file_size: DEFAULT_MAX_FILE_SIZE,

            buffer_len: DEFAULT_BUFFER_LEN,
            main_loop_sleep: DEFAULT_SLEEP_MAIN_LOOP,

            sync_policy: DEFAULT_SYNC_POLICY,

            min_level: None,
            poll_interval: None,
            runtime_level_file: None,

            run_thread_init: Box::new(|| ()),
        }
    }
}

impl LoggerBuilder {
    /// Directory holding the log files, and their base name:
    /// `<folder>/<UTC date>/<base>_<n>.log`. Default: `./.logs` and `log`.
    pub fn output(mut self, path_folder: impl Into<PathBuf>, base_name: impl Into<String>) -> Self {
        self.folder_name = path_folder.into();
        self.base_file_name = base_name.into();
        self
    }

    /// Places `folder_name` under the directory read from the environment
    /// variable `var_name` at runtime, or under the current directory if it is
    /// unset.
    ///
    /// This is the entry point for a deployed service: an operator or a
    /// container sets one variable, and nothing about the build tree ends up
    /// baked into the binary.
    pub fn output_from_env(
        self,
        var_name: &str,
        folder_name: impl AsRef<Path>,
        base_name: impl Into<String>,
    ) -> Self {
        match std::env::var_os(var_name) {
            Some(dir) => self.output(PathBuf::from(dir).join(folder_name), base_name),
            None => self.output(PathBuf::from(".").join(folder_name), base_name),
        }
    }

    /// Whether to create one sub-folder per UTC day. Default: `true`.
    pub fn include_date(mut self, include: bool) -> Self {
        self.include_date = include;
        self
    }
    /// Rotates once the current file would exceed this many bytes.
    /// Default: 10 MiB.
    pub fn max_file_size(mut self, bytes: u64) -> Self {
        self.max_file_size = bytes;
        self
    }
    /// How long the I/O thread sleeps when it finds nothing to write.
    /// Default: 10 ms.
    pub fn main_loop_sleep(mut self, sleep: Duration) -> Self {
        self.main_loop_sleep = sleep;
        self
    }
    /// When to force the data to disk. Default: on every message at
    /// [`LogLevel::Error`] or above.
    pub fn sync_policy(mut self, policy: SyncPolicy) -> Self {
        self.sync_policy = policy;
        self
    }
    /// In-memory write buffer size, in bytes. Default: 64 KiB.
    ///
    /// Zero means "use the default" rather than "no buffering": a zero-length
    /// buffer would flush on every message, which is the opposite of the point.
    pub fn buffer_len(mut self, bytes: u64) -> Self {
        self.buffer_len = if bytes == 0 {
            DEFAULT_BUFFER_LEN
        } else {
            bytes
        };
        self
    }

    /// Minimum level at startup. Default: [`LogLevel::Info`].
    ///
    /// Takes precedence over [`Self::runtime_level_file`] and **overwrites that
    /// file** with this value, so the code stays the single source of truth
    /// across restarts. Omit it if the operator's on-disk setting should
    /// survive a restart instead.
    pub fn min_level(mut self, level: LogLevel) -> Self {
        self.min_level = Some(level);
        self
    }
    /// File watched at runtime to change the minimum level without a restart.
    ///
    /// It holds a single level name, case-insensitive:
    ///
    /// ```text
    /// WARN
    /// ```
    ///
    /// A missing file is created with the starting level.
    pub fn runtime_level_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.runtime_level_file = Some(path.into());
        self
    }
    /// How often the control file is polled. Default: 60 s.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = Some(interval);
        self
    }
    /// Runs once on the logger thread before it starts (e.g. to set its CPU
    /// affinity).
    pub fn thread_init(mut self, init: impl FnOnce() + Send + 'static) -> Self {
        self.run_thread_init = Box::new(init);
        self
    }

    /// Starts the logger.
    ///
    /// Succeeds at most once per process: the sender is published into a
    /// `OnceLock` and deliberately never reclaimed, which is what lets `log`
    /// take it without a lock. A second call, including after
    /// [`Logger::shutdown`], returns [`Error::AlreadyInitialized`] without
    /// touching the running logger, its level or its files.
    ///
    /// If the I/O thread cannot be spawned, the call fails with
    /// [`Error::FailedToSpawnThread`] and the process is left without a
    /// logger: messages then go to stderr.
    pub fn build(self) -> Result<Logger, Error> {
        if self.buffer_len > self.max_file_size {
            return Err(Error::BufferBiggerThanFile {
                buffer_size: self.buffer_len,
                file_size: self.max_file_size,
            });
        }
        // The common case, checked before any side effect. The race between
        // two concurrent builds is settled by `logger::init` below.
        if logger::is_initialized() {
            return Err(Error::AlreadyInitialized);
        }

        let (sender, receiver) = mpsc::channel::<LogCommand, CHANNEL_LEN>();
        let file_manager = FileManager::new(
            self.folder_name.clone(),
            self.base_file_name,
            self.max_file_size,
            self.buffer_len,
            self.include_date,
            self.sync_policy,
        )
        .map_err(|source| Error::OpenLogFile {
            path: self.folder_name,
            source,
        })?;

        // Claim the singleton before starting a worker, so that a build losing
        // the race never runs a second worker on the same file.
        if logger::init(sender).is_err() {
            return Err(Error::AlreadyInitialized);
        }

        let initial = match &self.runtime_level_file {
            Some(path) => resolve_min_level(path, self.min_level),
            None => self.min_level.unwrap_or(DEFAULT_MIN_LEVEL),
        };
        min_level::set(initial);

        let handle = std::thread::Builder::new()
            .name(DEFAULT_THREAD_NAME.to_string())
            .spawn(move || {
                (self.run_thread_init)();
                let mut worker = LogWorker::new(
                    file_manager,
                    self.main_loop_sleep,
                    receiver,
                    self.runtime_level_file,
                    self.poll_interval.unwrap_or(DEFAULT_POLL_LEVEL_INTERVAL),
                );
                worker.run();
            })
            .map_err(Error::FailedToSpawnThread)?;

        Ok(Logger::new(handle))
    }
}
impl std::fmt::Debug for LoggerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoggerBuilder")
            .field("folder_name", &self.folder_name.display())
            .field("base_file_name", &self.base_file_name)
            .field("include_date", &self.include_date)
            .field("max_file_size", &self.max_file_size)
            .field("buffer_len", &self.buffer_len)
            .field("main_loop_sleep", &self.main_loop_sleep)
            .field("sync_policy", &self.sync_policy)
            .field("min_level", &self.min_level)
            .field("runtime_level_file", &self.runtime_level_file)
            .field("poll_interval", &self.poll_interval)
            .finish_non_exhaustive()
    }
}

/// The starting level, and what the control file says from now on.
fn resolve_min_level(path: &Path, explicit: Option<LogLevel>) -> LogLevel {
    // A level set in code is authoritative: persist it to the control file.
    if let Some(level) = explicit {
        if let Err(e) = std::fs::write(path, level.as_str()) {
            eprintln!("[LOGGER] cannot write the level to {}: {e}", path.display());
        }
        return level;
    }
    // Otherwise the file is authoritative.
    match std::fs::read_to_string(path) {
        Ok(s) => LogLevel::from_str(&s).unwrap_or_else(|| {
            eprintln!(
                "[LOGGER] '{}' in {} is not a log level, falling back to {DEFAULT_MIN_LEVEL}",
                s.trim(),
                path.display(),
            );
            DEFAULT_MIN_LEVEL
        }),
        Err(_) => {
            // Missing or unreadable: seed it with the default.
            if let Err(e) = std::fs::write(path, DEFAULT_MIN_LEVEL.as_str()) {
                eprintln!(
                    "[LOGGER] cannot write {DEFAULT_MIN_LEVEL} to {}: {e}",
                    path.display()
                );
            }
            DEFAULT_MIN_LEVEL
        }
    }
}

#[cfg(test)]
#[path = "tests/builder.rs"]
mod builder_test;
