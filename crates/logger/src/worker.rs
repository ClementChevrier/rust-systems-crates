use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

use channel::mpsc;

use super::{
    config::{logger::CHANNEL_LEN, state::min_level},
    file_manager::FileManager,
    message::{LogCommand, LogLevel, LogMessage},
    metrics,
};

/// Owns the file and the write buffer on a dedicated thread.
///
/// Error policy: write failures are reported to stderr and the worker keeps
/// running, since a logger must never crash the application it observes.
pub(crate) struct LogWorker {
    output: FileManager,
    main_loop_sleep: Duration,
    receiver: mpsc::Consumer<LogCommand, { CHANNEL_LEN }>,

    // Runtime change of the minimum level.
    min_level_file: Option<PathBuf>,
    last_runtime_modif: Option<SystemTime>,
    poll_interval: Duration,
    // Set while the control file cannot be read, so the failure is reported
    // once rather than at every poll.
    level_file_unreadable: bool,
}

impl LogWorker {
    pub(super) fn new(
        output: FileManager,
        main_loop_sleep: Duration,
        receiver: mpsc::Consumer<LogCommand, { CHANNEL_LEN }>,
        min_level_file: Option<PathBuf>,
        poll_interval: Duration,
    ) -> Self {
        Self {
            output,
            main_loop_sleep,
            receiver,

            min_level_file,
            last_runtime_modif: None,
            poll_interval,
            level_file_unreadable: false,
        }
    }

    pub(super) fn run(&mut self) {
        self.output.write(LogMessage::start());
        let mut last_level_check = Instant::now();

        'work_loop: loop {
            let mut did_work = false;
            for msg in self.receiver.drain() {
                did_work = true;
                match msg {
                    LogCommand::Message(msg) => self.output.write(msg),
                    LogCommand::Shutdown => {
                        self.output.write(LogMessage::shutdown());
                        self.shutdown();
                        break 'work_loop;
                    }
                }
            }

            self.report_drops();

            if self.receiver.is_closed() {
                self.output.write(LogMessage::senders_dead());
                self.shutdown();
                break;
            }

            if last_level_check.elapsed() >= self.poll_interval {
                self.poll_min_level();
                last_level_check = Instant::now();
            }

            if !did_work {
                std::thread::sleep(self.main_loop_sleep);
            }
        }
    }

    fn poll_min_level(&mut self) {
        let Some(path) = &self.min_level_file else {
            return;
        };

        // Skip the read when the modification time is unchanged. If it cannot
        // be read, fall through and read the file anyway.
        if let Ok(modified) = fs::metadata(path).and_then(|m| m.modified()) {
            if Some(modified) == self.last_runtime_modif {
                return;
            }
            self.last_runtime_modif = Some(modified);
        }

        let content = match fs::read_to_string(path) {
            Ok(content) => {
                self.level_file_unreadable = false;
                content
            }
            Err(e) => {
                // The file may legitimately be removed while running: keep the
                // current level, and complain only once, on the transition.
                if !self.level_file_unreadable {
                    eprintln!("[LOGGER] cannot read {}: {e}", path.display());
                    self.level_file_unreadable = true;
                }
                return;
            }
        };

        match LogLevel::from_str(&content) {
            Some(level) => min_level::set(level),
            None => eprintln!("[LOGGER] '{}' is not a log level", content.trim()),
        }
    }

    /// Writes down the messages dropped since the last report, so the loss is
    /// visible in the log itself and not only in the metrics.
    fn report_drops(&mut self) {
        let dropped = metrics::take_unreported_drops();
        if dropped > 0 {
            self.output.write(LogMessage::dropped(dropped));
        }
    }

    fn shutdown(&mut self) {
        // One last drain, so nothing already queued is lost.
        for msg in self.receiver.drain() {
            if let LogCommand::Message(msg) = msg {
                self.output.write(msg)
            }
        }
        self.report_drops();
        self.output.shutdown();
    }
}

impl Drop for LogWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
#[path = "tests/worker.rs"]
mod worker_test;
