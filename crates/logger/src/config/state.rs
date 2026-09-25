use std::sync::atomic::{AtomicU8, Ordering};

use channel::mpsc::Producer;

use super::{
    super::message::{LogCommand, LogLevel},
    builder::DEFAULT_MIN_LEVEL,
    logger::CHANNEL_LEN,
};

pub(crate) mod logger {
    use super::*;
    use std::sync::OnceLock;

    type Sender = Producer<LogCommand, CHANNEL_LEN>;

    static LOGGER: OnceLock<Sender> = OnceLock::new();
    pub(crate) fn init(sender: Sender) -> Result<(), Sender> {
        LOGGER.set(sender)
    }
    pub(crate) fn with<R>(func: impl FnOnce(&Sender) -> R) -> Option<R> {
        LOGGER.get().filter(|s| !s.is_closed()).map(func)
    }
    pub(crate) fn is_initialized() -> bool {
        LOGGER.get().is_some()
    }
    pub(crate) fn close() {
        if let Some(s) = LOGGER.get() {
            s.close();
        }
    }
}

pub(crate) mod min_level {
    use super::*;

    static MIN_LEVEL: AtomicU8 = AtomicU8::new(DEFAULT_MIN_LEVEL.as_int());
    pub(crate) fn set(level: LogLevel) {
        MIN_LEVEL.store(level.as_int(), Ordering::Relaxed);
    }
    pub(crate) fn get() -> u8 {
        MIN_LEVEL.load(Ordering::Relaxed)
    }
}
