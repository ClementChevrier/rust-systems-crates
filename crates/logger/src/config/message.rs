/// Maximum length of a log title, in bytes. The `log_*` macros reject a longer
/// title at compile time. Titles are written as they are, without padding.
pub const TITLE_WIDTH: usize = 15;

pub(crate) const BASE_BODY_CAPACITY: usize = 256;

/// "YYYY-MM-DD HH:MM:SS.nnnnnnnnn"
pub(crate) const TIMESTAMP_LEN: usize = 29;
