use std::time::Duration;

pub(crate) const CHANNEL_LEN: usize = 1024;
pub(crate) const SHUTDOWN_SEND_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const SHUTDOWN_SEND_RETRY: Duration = Duration::from_millis(1);
