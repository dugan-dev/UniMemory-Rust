use std::fmt;

/// Allocation and resource-management errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The backend or fixed buffer could not satisfy the allocation.
    OutOfMemory,
    /// A requested size or alignment cannot form a valid Rust Layout.
    InvalidLayout,
    /// Statistics support was disabled when building the crate.
    StatisticsDisabled,
    /// Tracked allocations must be released before changing statistics.
    Busy,
    /// A checkpoint belongs to another Stack or an expired generation.
    InvalidMark,
    /// A native allocator rejected a control operation.
    Backend(&'static str),
}

/// A fallible UniMemory operation.
pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::OutOfMemory => "allocation failed or buffer exhausted",
            Self::InvalidLayout => "invalid allocation layout",
            Self::StatisticsDisabled => "compile with the statistics feature",
            Self::Busy => "release tracked allocations before changing statistics",
            Self::InvalidMark => "invalid or expired stack checkpoint",
            Self::Backend(message) => message,
        })
    }
}

impl std::error::Error for Error {}
