//! The `log_*` macros, one per [`LogLevel`](crate::LogLevel).
//!
//! Each takes a title, a string literal of at most
//! [`TITLE_WIDTH`](crate::TITLE_WIDTH) bytes (checked at compile time), then a
//! `format!`-style message. The level check comes first, so a filtered message
//! is never formatted.

#[doc(hidden)]
#[macro_export]
macro_rules! __logger_log {
    ($level:expr, $component:literal, $($arg:tt)*) => {{
        const {
            assert!(
                $component.len() <= $crate::__private::TITLE_WIDTH,
                "log title exceeds TITLE_WIDTH"
            )
        };
        if $crate::__private::allowed($level) {
            $crate::__private::log($level, $component, format_args!($($arg)*));
        }
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! __logger_log_debug {
    ($($arg:tt)*) => { $crate::__logger_log!($crate::LogLevel::Debug, $($arg)*) };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __logger_log_info {
    ($($arg:tt)*) => { $crate::__logger_log!($crate::LogLevel::Info, $($arg)*) };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __logger_log_warn {
    ($($arg:tt)*) => { $crate::__logger_log!($crate::LogLevel::Warn, $($arg)*) };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __logger_log_error {
    ($($arg:tt)*) => { $crate::__logger_log!($crate::LogLevel::Error, $($arg)*) };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __logger_log_critical {
    ($($arg:tt)*) => { $crate::__logger_log!($crate::LogLevel::Critical, $($arg)*) };
}

/// Logs at [`LogLevel::Critical`](crate::LogLevel::Critical):
/// `log_critical!("TITLE", "format {}", args)`.
pub use crate::__logger_log_critical as log_critical;
/// Logs at [`LogLevel::Debug`](crate::LogLevel::Debug):
/// `log_debug!("TITLE", "format {}", args)`.
pub use crate::__logger_log_debug as log_debug;
/// Logs at [`LogLevel::Error`](crate::LogLevel::Error):
/// `log_error!("TITLE", "format {}", args)`.
pub use crate::__logger_log_error as log_error;
/// Logs at [`LogLevel::Info`](crate::LogLevel::Info):
/// `log_info!("TITLE", "format {}", args)`.
pub use crate::__logger_log_info as log_info;
/// Logs at [`LogLevel::Warn`](crate::LogLevel::Warn):
/// `log_warn!("TITLE", "format {}", args)`.
pub use crate::__logger_log_warn as log_warn;
