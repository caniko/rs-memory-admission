//! Built-in [`MemoryProvider`] implementations.
//!
//! [`MemoryProvider`]: crate::MemoryProvider

use std::sync::Arc;

use crate::provider::{MemoryProvider, MemoryStats, ProviderError, SharedMemoryProvider};

/// Select the preferred memory provider for the current platform.
///
/// Provider precedence is:
///
/// 1. Linux cgroup-v2 accounting when the current process is in a cgroup-v2
///    hierarchy.
/// 2. Linux `/proc/meminfo`.
/// 3. `sysinfo` when the `sysinfo` feature is enabled.
///
/// On unsupported target/feature combinations this returns a provider that
/// reports [`ProviderError::Unsupported`], allowing gates to fall back to
/// thread-cap-only operation.
#[must_use]
pub fn default_provider() -> SharedMemoryProvider {
    #[cfg(target_os = "linux")]
    {
        if let Some(provider) = CgroupV2Provider::shared() {
            provider
        } else {
            ProcMeminfoProvider::shared()
        }
    }

    #[cfg(all(not(target_os = "linux"), feature = "sysinfo"))]
    {
        SysinfoProvider::shared()
    }

    #[cfg(all(not(target_os = "linux"), not(feature = "sysinfo")))]
    {
        Arc::new(UnsupportedProvider)
    }
}

#[cfg(all(not(target_os = "linux"), not(feature = "sysinfo")))]
struct UnsupportedProvider;

#[cfg(all(not(target_os = "linux"), not(feature = "sysinfo")))]
impl MemoryProvider for UnsupportedProvider {
    fn used_fraction(&self) -> Result<f64, ProviderError> {
        Err(ProviderError::Unsupported)
    }
}

/// Reads `/proc/meminfo` on Linux and returns `1.0 - MemAvailable/MemTotal`.
///
/// This is cheap (a single short file read), always-current (kernel updates
/// the file on every read), and unaffected by per-process memory accounting
/// quirks. It is the recommended provider on Linux.
pub struct ProcMeminfoProvider;

impl ProcMeminfoProvider {
    /// Path read by this provider.
    pub const PATH: &'static str = "/proc/meminfo";

    /// Construct an `Arc` ready to hand to a gate.
    #[must_use]
    pub fn shared() -> SharedMemoryProvider {
        Arc::new(Self)
    }
}

impl MemoryProvider for ProcMeminfoProvider {
    fn used_fraction(&self) -> Result<f64, ProviderError> {
        Ok(self.stats()?.used_fraction())
    }

    fn stats(&self) -> Result<MemoryStats, ProviderError> {
        let contents = std::fs::read_to_string(Self::PATH)
            .map_err(|e| ProviderError::new(format!("read {}: {e}", Self::PATH)))?;
        parse_proc_meminfo_stats(&contents)
    }
}

/// Reads cgroup-v2 memory accounting for the calling process.
///
/// When the application runs inside a `systemd-run --scope -p MemoryHigh=…`
/// (or any other cgroup with a memory limit), the *host* `MemAvailable` is
/// no longer the relevant signal — the cgroup will be throttled long before
/// host memory is exhausted, and a host-level gate will happily admit work
/// that the kernel then forces into swap.
///
/// This provider locates the calling process's cgroup-v2 path (from
/// `/proc/self/cgroup`), then reports:
///
/// - `total_bytes` — the cgroup's `memory.high` (or `memory.max` if `high`
///   is not set), falling back to host total when neither is configured.
/// - `available_bytes` — `total_bytes - memory.current`, clamped to zero.
/// - `page_cache_bytes` — `memory.stat`'s `file` accounting.
///
/// Linux-only. Use [`ProcMeminfoProvider`] or [`SysinfoProvider`] when the
/// host's view is the authoritative one.
#[cfg(target_os = "linux")]
pub struct CgroupV2Provider {
    base_dir: std::path::PathBuf,
    fallback_total: u64,
}

#[cfg(target_os = "linux")]
impl CgroupV2Provider {
    /// Build a provider for the calling process's cgroup. Returns `None` if
    /// the process is not in a cgroup-v2 hierarchy (e.g. legacy cgroup-v1).
    #[must_use]
    pub fn for_self() -> Option<Self> {
        let cgroup_line = std::fs::read_to_string("/proc/self/cgroup").ok()?;
        // cgroup-v2 line format: `0::/user.slice/...` (single line, prefix `0::`)
        let path = cgroup_line.lines().find_map(|l| l.strip_prefix("0::"))?;
        let base_dir = std::path::Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
        if !base_dir.is_dir() {
            return None;
        }
        // Use the host total as the fallback when no cgroup limit is set.
        let fallback_total = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| parse_memtotal_kib(&s))
            .map(|kib| kib.saturating_mul(1024))
            .unwrap_or(0);
        Some(Self {
            base_dir,
            fallback_total,
        })
    }

    /// Wrap in an `Arc` for handoff to a gate.
    #[must_use]
    pub fn shared() -> Option<SharedMemoryProvider> {
        Self::for_self().map(|p| Arc::new(p) as SharedMemoryProvider)
    }

    fn read_limit(&self, name: &str) -> Option<u64> {
        let raw = std::fs::read_to_string(self.base_dir.join(name)).ok()?;
        let trimmed = raw.trim();
        if trimmed == "max" || trimmed.is_empty() {
            return None;
        }
        trimmed.parse().ok()
    }

    fn read_current(&self) -> Option<u64> {
        std::fs::read_to_string(self.base_dir.join("memory.current"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    fn read_file_cache(&self) -> u64 {
        let Ok(stat) = std::fs::read_to_string(self.base_dir.join("memory.stat")) else {
            return 0;
        };
        parse_cgroup_file_cache_bytes(&stat)
    }
}

#[cfg(target_os = "linux")]
impl MemoryProvider for CgroupV2Provider {
    fn used_fraction(&self) -> Result<f64, ProviderError> {
        Ok(self.stats()?.used_fraction())
    }

    fn stats(&self) -> Result<MemoryStats, ProviderError> {
        let current = self
            .read_current()
            .ok_or_else(|| ProviderError::new("cgroup memory.current unreadable"))?;
        // Prefer `memory.high` over `memory.max`: high is the soft cap that
        // triggers throttling and matches what `systemd-run -p MemoryHigh=…`
        // sets. `max` is the OOM ceiling, usually higher.
        let total = self
            .read_limit("memory.high")
            .or_else(|| self.read_limit("memory.max"))
            .filter(|&v| v > 0)
            .unwrap_or(self.fallback_total);
        if total == 0 {
            return Err(ProviderError::new(
                "no usable cgroup memory limit and no host fallback total",
            ));
        }
        let available = total.saturating_sub(current);
        Ok(MemoryStats {
            total_bytes: total,
            available_bytes: available,
            page_cache_bytes: self.read_file_cache(),
        })
    }
}

/// Returns a fixed value (useful for tests).
#[derive(Debug, Clone)]
pub struct FixedProvider {
    /// The constant fraction returned by [`MemoryProvider::used_fraction`].
    pub fraction: f64,
}

impl FixedProvider {
    /// Create a fixed-value provider.
    #[must_use]
    pub const fn new(fraction: f64) -> Self {
        Self { fraction }
    }

    /// Wrap in an `Arc` for handoff to a gate.
    #[must_use]
    pub fn shared(fraction: f64) -> SharedMemoryProvider {
        Arc::new(Self::new(fraction))
    }
}

impl MemoryProvider for FixedProvider {
    fn used_fraction(&self) -> Result<f64, ProviderError> {
        Ok(self.fraction)
    }

    fn stats(&self) -> Result<MemoryStats, ProviderError> {
        // Synthesize plausible stats so weighted gate tests can drive the
        // FixedProvider end-to-end.
        const ONE_GIB: u64 = 1024 * 1024 * 1024;
        let total = ONE_GIB;
        let available = (((1.0 - self.fraction).max(0.0)) * total as f64) as u64;
        Ok(MemoryStats {
            total_bytes: total,
            available_bytes: available.min(total),
            page_cache_bytes: 0,
        })
    }
}

/// Cross-platform memory provider backed by the `sysinfo` crate.
///
/// Available when the `sysinfo` feature is enabled (default).
#[cfg(feature = "sysinfo")]
pub struct SysinfoProvider {
    system: std::sync::Mutex<sysinfo::System>,
}

#[cfg(feature = "sysinfo")]
impl SysinfoProvider {
    /// Build a fresh sysinfo provider with memory refresh enabled.
    #[must_use]
    pub fn new() -> Self {
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        Self {
            system: std::sync::Mutex::new(system),
        }
    }

    /// Wrap in an `Arc` for handoff to a gate.
    #[must_use]
    pub fn shared() -> SharedMemoryProvider {
        Arc::new(Self::new())
    }
}

#[cfg(feature = "sysinfo")]
impl Default for SysinfoProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "sysinfo")]
impl MemoryProvider for SysinfoProvider {
    fn used_fraction(&self) -> Result<f64, ProviderError> {
        Ok(self.stats()?.used_fraction())
    }

    fn stats(&self) -> Result<MemoryStats, ProviderError> {
        let mut system = self
            .system
            .lock()
            .map_err(|e| ProviderError::new(format!("sysinfo mutex poisoned: {e}")))?;
        system.refresh_memory();
        let total = system.total_memory();
        if total == 0 {
            return Err(ProviderError::new("sysinfo reports total_memory = 0"));
        }
        let available = system.available_memory().min(total);
        Ok(MemoryStats {
            total_bytes: total,
            available_bytes: available,
            page_cache_bytes: 0,
        })
    }
}

#[cfg(test)]
fn parse_proc_meminfo(contents: &str) -> Result<f64, ProviderError> {
    Ok(parse_proc_meminfo_stats(contents)?.used_fraction())
}

fn parse_proc_meminfo_stats(contents: &str) -> Result<MemoryStats, ProviderError> {
    let mut total_kib = None;
    let mut available_kib = None;
    let mut buffers_kib: u64 = 0;
    let mut cached_kib: u64 = 0;

    for line in contents.lines() {
        if let Some(v) = parse_meminfo_line(line, "MemTotal") {
            total_kib = Some(v);
        } else if let Some(v) = parse_meminfo_line(line, "MemAvailable") {
            available_kib = Some(v);
        } else if let Some(v) = parse_meminfo_line(line, "Buffers") {
            buffers_kib = v;
        } else if let Some(v) = parse_meminfo_line(line, "Cached") {
            cached_kib = v;
        }
    }

    let total = total_kib.ok_or_else(|| ProviderError::new("MemTotal not found"))?;
    let available = available_kib.ok_or_else(|| ProviderError::new("MemAvailable not found"))?;

    if total == 0 {
        return Err(ProviderError::new("MemTotal must be > 0"));
    }
    if available > total {
        return Err(ProviderError::new("MemAvailable cannot exceed MemTotal"));
    }

    Ok(MemoryStats {
        total_bytes: total.saturating_mul(1024),
        available_bytes: available.saturating_mul(1024),
        page_cache_bytes: buffers_kib.saturating_add(cached_kib).saturating_mul(1024),
    })
}

fn parse_meminfo_line(line: &str, key: &str) -> Option<u64> {
    let (name, rest) = line.split_once(':')?;
    if name.trim() != key {
        return None;
    }
    rest.split_whitespace().next()?.parse::<u64>().ok()
}

#[cfg(target_os = "linux")]
fn parse_memtotal_kib(contents: &str) -> Option<u64> {
    contents
        .lines()
        .find_map(|line| parse_meminfo_line(line, "MemTotal"))
}

#[cfg(target_os = "linux")]
fn parse_cgroup_file_cache_bytes(contents: &str) -> u64 {
    contents
        .lines()
        .find_map(|line| {
            let rest = line.strip_prefix("file ")?;
            rest.trim().parse::<u64>().ok()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_proc_meminfo() {
        let contents = "MemTotal:       65536000 kB\nMemFree:         1024000 kB\nMemAvailable:   13107200 kB\n";
        let used = parse_proc_meminfo(contents).unwrap();
        assert!((used - 0.80).abs() < 0.01);
    }

    #[test]
    fn errors_when_available_exceeds_total() {
        let contents = "MemTotal: 1000 kB\nMemAvailable: 2000 kB\n";
        assert!(parse_proc_meminfo(contents).is_err());
    }

    #[test]
    fn errors_when_total_zero() {
        let contents = "MemTotal: 0 kB\nMemAvailable: 0 kB\n";
        assert!(parse_proc_meminfo(contents).is_err());
    }

    #[test]
    fn errors_when_field_missing() {
        let contents = "MemTotal: 1000 kB\n";
        assert!(parse_proc_meminfo(contents).is_err());
    }

    #[test]
    fn default_provider_constructs() {
        let _provider = default_provider();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_cgroup_file_cache() {
        let contents = "anon 1024\nfile 4096\nkernel 512\n";
        assert_eq!(parse_cgroup_file_cache_bytes(contents), 4096);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_memtotal_for_cgroup_fallback() {
        let contents = "MemFree: 100 kB\nMemTotal: 2048 kB\n";
        assert_eq!(parse_memtotal_kib(contents), Some(2048));
    }
}
