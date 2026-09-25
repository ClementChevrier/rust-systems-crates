use std::{
    error::Error,
    fmt::{Debug, Display},
};

/// Why a ring could not be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    /// A ring must hold at least one element.
    ZeroCapacity,
    /// The next power of two above the requested capacity does not fit in a
    /// `usize`.
    CapacityOverflow {
        /// The capacity that was asked for.
        asked_cap: usize,
    },
}

impl Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroCapacity => f.write_str("requested capacity is zero"),
            Self::CapacityOverflow { asked_cap } => write!(
                f,
                "requested capacity {asked_cap} rounds up to a power of two larger than usize::MAX"
            ),
        }
    }
}

impl Error for InitError {}
