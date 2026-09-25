//! Keeping memory resident.
//!
//! A buffer spread over several pages can have any of them evicted to the page
//! file, and the next access then stalls on a hard page fault. [`lock`] and
//! [`lock_mut`] pin those pages into RAM for as long as the returned guard
//! lives.

mod lock;
pub use lock::*;
