use std::{
    thread::JoinHandle,
    time::{Instant, SystemTime},
};

use channel::Error as ChannelError;

use super::{
    builder::LoggerBuilder,
    config::{
        logger::*,
        message::BASE_BODY_CAPACITY,
        state::{logger, min_level},
    },
    error::Error,
    message::*,
    metrics::{self, LoggerMetrics},
};

/// Handle on the running logger.
///
/// Logging itself goes through the `log_*` macros and needs no handle; this
/// one exists to shut the I/O thread down, explicitly or on drop.
#[derive(Debug)]
pub struct Logger {
    handle: Option<JoinHandle<()>>,
}

impl Logger {
    pub(crate) fn new(handle: JoinHandle<()>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    /// Starts a logger with every default: `./.logs/<UTC date>/log_<n>.log`.
    pub fn start_default() -> Result<Self, Error> {
        LoggerBuilder::default().build()
    }

    /// A builder to configure the logger before starting it.
    pub fn builder() -> LoggerBuilder {
        LoggerBuilder::default()
    }

    /// A non-destructive read of the process-wide counters.
    ///
    /// Associated function rather than a method: the counters are process-global,
    /// so this works without holding the `Logger`.
    ///
    /// Beware when computing a rate. A rotation between two calls writes the
    /// metrics footer, which resets every counter, so the second reading can be
    /// lower than the first. Use `saturating_sub`, and treat a drop as "a rotation
    /// happened" rather than as an error.
    pub fn peek_metrics() -> LoggerMetrics {
        metrics::peek()
    }

    /// True if the order was delivered, or if the worker is already gone (the
    /// join then returns immediately).
    fn send_shutdown_to_worker(&mut self) -> bool {
        let deadline = Instant::now() + SHUTDOWN_SEND_TIMEOUT;
        loop {
            match logger::with(|sender| sender.push(LogCommand::Shutdown)) {
                Some(Ok(())) => return true,
                None | Some(Err(ChannelError::ReceiverDisconnected(_))) => {
                    eprintln!("[LOGGER] worker already dead, nothing to shut down");
                    return true;
                }
                Some(Err(ChannelError::ChannelFull(_))) => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(SHUTDOWN_SEND_RETRY);
                }
            }
        }
    }

    /// Flushes, writes the shutdown marker and joins the thread. Idempotent;
    /// also called by `Drop`.
    ///
    /// If the order cannot be delivered within a timeout (a frozen disk, for
    /// instance), the worker is detached rather than joined, and stderr says
    /// that buffered lines may be lost.
    pub fn shutdown(&mut self) {
        if self.handle.is_none() {
            return;
        }

        if !self.send_shutdown_to_worker() {
            // The worker is stuck (frozen I/O?): joining a thread that will
            // never receive the order would block the whole process. Detach it.
            eprintln!(
                "[LOGGER] shutdown not delivered after {SHUTDOWN_SEND_TIMEOUT:?}: worker left detached, buffered logs may be lost"
            );
            logger::close();
            self.handle.take();
            return;
        }

        logger::close();
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            eprintln!("[LOGGER] logger thread panicked before shutdown completed");
        }
    }
}

impl Drop for Logger {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Formats and enqueues one message. Called by the `log_*` macros.
///
/// Never blocks: when the channel is full the message is dropped and counted.
/// Before [`LoggerBuilder::build`], or after a shutdown, the message goes to
/// stderr instead.
pub fn log(level: LogLevel, title: &'static str, args: std::fmt::Arguments<'_>) {
    let mut body = String::with_capacity(BASE_BODY_CAPACITY);
    // Writing into a `String` cannot fail.
    let _ = std::fmt::Write::write_fmt(&mut body, args);
    let command = LogCommand::Message(LogMessage {
        level,
        ts: SystemTime::now(),
        title,
        body: body.into_bytes(),
    });

    match logger::with(|sender| sender.push(command)) {
        Some(Ok(())) => metrics::record_enqueued(),
        Some(Err(ChannelError::ChannelFull(command))) => {
            if let LogCommand::Message(message) = command {
                metrics::record_drop(message.len());
            }
        }
        // No logger yet, or it closed between the check and the push: the
        // message can no longer reach a file, and stderr is the only place left.
        None | Some(Err(ChannelError::ReceiverDisconnected(_))) => {
            eprintln!("[LOGGER] no active logger: [{title}] {args}");
        }
    }
}

/// True when a message at `level` passes the current minimum level. Called by
/// the `log_*` macros before anything is formatted.
#[inline(always)]
pub fn allowed(level: LogLevel) -> bool {
    level.as_int() >= min_level::get()
}
