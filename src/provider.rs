//! Memory provider abstractions and error types.

use std::sync::Arc;

/// Reads the host's current memory state.
///
/// `used_fraction` returns a value in the closed range `[0.0, 1.0]`. The
/// optional [`MemoryProvider::stats`] returns absolute byte counts for
/// budget-aware (weighted) admission. Errors are reported as a
/// [`ProviderError`] string; the gate logs and disables the memory scheduler
/// the first time a provider error appears.
pub trait MemoryProvider: Send + Sync + 'static {
    /// Probe the current used-RAM fraction.
    ///
    /// # Errors
    /// Returns [`ProviderError`] when the underlying source is unavailable or
    /// returns inconsistent values (e.g. `MemAvailable > MemTotal`).
    fn used_fraction(&self) -> Result<f64, ProviderError>;

    /// Probe absolute memory totals.
    ///
    /// The default implementation returns `Err(ProviderError::Unsupported)`.
    /// Providers that can supply absolute counts (`/proc/meminfo`, sysinfo)
    /// override this so the weighted gates can budget by bytes rather than
    /// just fraction.
    ///
    /// # Errors
    /// Returns [`ProviderError`] when stats cannot be read.
    ///
    fn stats(&self) -> Result<MemoryStats, ProviderError> {
        Err(ProviderError::Unsupported)
    }
}

/// Absolute memory state in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryStats {
    /// Total physical memory.
    pub total_bytes: u64,
    /// Memory considered available for new allocations without swapping.
    pub available_bytes: u64,
    /// Bytes currently held in the kernel page cache (Linux: `Buffers + Cached`).
    /// Zero when the underlying provider does not report this separately.
    ///
    /// Tracked because a large page cache combined with heavy I/O causes the
    /// reclaim path itself (`kswapd`) to become a bottleneck — even though
    /// `MemAvailable` is optimistic about how cheaply that cache can be
    /// dropped.
    pub page_cache_bytes: u64,
}

impl MemoryStats {
    /// Used fraction `1.0 - available/total`. Returns 0.0 if `total_bytes` is 0.
    #[must_use]
    pub fn used_fraction(self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            1.0 - (self.available_bytes as f64 / self.total_bytes as f64)
        }
    }
}

/// Boxed memory provider — type alias for ergonomic storage in gates.
pub type SharedMemoryProvider = Arc<dyn MemoryProvider>;

/// Error produced by a [`MemoryProvider`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Underlying source is unavailable, malformed, or inconsistent.
    Source(String),
    /// The provider does not support the requested operation
    /// (e.g. [`MemoryProvider::stats`] when the implementation is
    /// fraction-only).
    Unsupported,
}

impl ProviderError {
    /// Construct a `Source` error from any displayable value.
    pub fn new(msg: impl Into<String>) -> Self {
        Self::Source(msg.into())
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(s) => f.write_str(s),
            Self::Unsupported => f.write_str("memory provider does not support this operation"),
        }
    }
}

impl std::error::Error for ProviderError {}

impl<F> MemoryProvider for F
where
    F: Fn() -> Result<f64, ProviderError> + Send + Sync + 'static,
{
    fn used_fraction(&self) -> Result<f64, ProviderError> {
        (self)()
    }
}
