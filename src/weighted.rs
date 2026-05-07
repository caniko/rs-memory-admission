//! Weighted (byte-budgeted) admission gates.
//!
//! Plain [`crate::sync::AdmissionGate`] / [`crate::r#async::AdmissionGate`]
//! treat every task as equal: as long as host RAM usage is below the threshold
//! they admit unboundedly many. That works when tasks have similar memory
//! cost. It breaks when a single pipeline mixes 5 KB metadata writes with
//! 10 GB archive extractions — the gate happily admits 64 large workers, the
//! kernel reports them as cache pressure on the next probe, by which point
//! several have already been spawned (and `7zz` subprocesses outside the
//! Tokio scheduler are immune to the gate's later throttling).
//!
//! The weighted admission gates solve this by:
//!
//! 1. Capturing the live `MemAvailable` at every probe.
//! 2. Tracking the *committed weight* — the sum of byte costs declared by
//!    permits currently in flight.
//! 3. Admitting only when `available_bytes` minus a configurable safety
//!    reserve, minus committed weight, is at least the new task's weight.
//!
//! The result is a hard byte budget that scales with the host instead of a
//! fixed worker count, and that accounts for subprocesses (since their
//! growing allocations show up as falling `MemAvailable` while their parent
//! still holds the permit).
//!
//! This module mirrors the API of [`crate::sync`] / [`crate::r#async`]: sync
//! and async admission gates plus a [`WeightedPermit`] that reserves its byte
//! count for the lifetime of the permit.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(feature = "async")]
use tokio::sync::Notify;

#[cfg(feature = "sync")]
use std::sync::Condvar;

use crate::config::{Config, ConfigError};
use crate::provider::{MemoryStats, ProviderError, SharedMemoryProvider};

/// Configuration for a weighted admission gate.
#[derive(Debug, Clone)]
pub struct WeightedConfig {
    /// Inherited base config (max_ram_fraction, hysteresis, poll_interval).
    pub base: Config,

    /// Bytes deliberately left unallocated as a safety margin. The gate will
    /// not commit weight that drives `available_bytes` below this floor.
    /// Defaults to 1 GiB.
    pub safety_reserve_bytes: u64,

    /// Hard upper bound on a single permit's weight, regardless of host
    /// availability. Tasks declaring more than this still receive a permit
    /// (the gate would otherwise deadlock on the first oversized task) but
    /// run alone — no other permits are admitted while one is in flight.
    /// Defaults to 8 GiB.
    pub max_single_weight_bytes: u64,

    /// Throttle when `Buffers + Cached` exceeds this fraction of total RAM,
    /// even if `MemAvailable` looks fine. Heavy I/O workloads (archive
    /// extractions, large file copies) build up readahead folios faster than
    /// the kernel reclaim path can drop them, leading to swap thrash even
    /// though `MemAvailable` reports headroom.
    ///
    /// Set to `1.0` to disable. Defaults to `0.6`.
    pub max_page_cache_fraction: f64,

    /// How long a stats probe is considered fresh. Stats are re-read more
    /// often than this only when waiting for admission. Defaults to 100 ms.
    pub stats_max_age: Duration,
}

impl Default for WeightedConfig {
    fn default() -> Self {
        Self {
            base: Config::default(),
            safety_reserve_bytes: 1 << 30,
            max_single_weight_bytes: 8 * (1 << 30),
            max_page_cache_fraction: 0.6,
            stats_max_age: Duration::from_millis(100),
        }
    }
}

impl WeightedConfig {
    /// Return the validated default configuration.
    #[must_use]
    pub fn validated_default() -> Self {
        Self::default()
            .validate()
            .expect("default weighted memory-admission config must be valid")
    }

    /// Validates the configuration. Returns `Ok(self)` if valid.
    ///
    /// # Errors
    /// Returns [`WeightedConfigError`] when any field is out of range.
    pub fn validate(self) -> Result<Self, WeightedConfigError> {
        self.base
            .clone()
            .validate()
            .map_err(WeightedConfigError::Base)?;

        if !(self.max_page_cache_fraction.is_finite()
            && self.max_page_cache_fraction > 0.0
            && self.max_page_cache_fraction <= 1.0)
        {
            return Err(WeightedConfigError::MaxPageCacheFractionOutOfRange(
                self.max_page_cache_fraction,
            ));
        }

        if self.stats_max_age.is_zero() {
            return Err(WeightedConfigError::StatsMaxAgeZero);
        }

        Ok(self)
    }
}

/// Errors produced when validating a [`WeightedConfig`].
#[derive(Debug, Clone, PartialEq)]
pub enum WeightedConfigError {
    /// Embedded base config is invalid.
    Base(ConfigError),
    /// `max_page_cache_fraction` must be finite and in `(0.0, 1.0]`.
    MaxPageCacheFractionOutOfRange(f64),
    /// `stats_max_age` must be non-zero.
    StatsMaxAgeZero,
}

impl std::fmt::Display for WeightedConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Base(e) => write!(f, "base config invalid: {e}"),
            Self::MaxPageCacheFractionOutOfRange(v) => write!(
                f,
                "max_page_cache_fraction must be finite and in (0.0, 1.0], got {v}"
            ),
            Self::StatsMaxAgeZero => f.write_str("stats_max_age must be > 0"),
        }
    }
}

impl std::error::Error for WeightedConfigError {}

#[derive(Debug)]
struct GateState {
    committed_bytes: u64,
    active_permits: usize,
    oversized_in_flight: bool,
    cached_stats: Option<MemoryStats>,
    cached_at: Option<Instant>,
    scheduler_active: bool,
    failure_logged: bool,
    /// Edge-triggered: true between entering and leaving the throttled state.
    /// Each transition logs once, avoiding the per-poll log flood that
    /// otherwise drowns the application's own progress output.
    throttled: bool,
}

struct Inner {
    state: Mutex<GateState>,
    #[cfg(feature = "sync")]
    condvar: Condvar,
    #[cfg(feature = "async")]
    notify: Notify,
    provider: SharedMemoryProvider,
    config: WeightedConfig,
}

/// Decision returned by the internal admit-attempt step.
enum AttemptOutcome {
    /// Permit granted; caller increments accounting and returns.
    Admitted,
    /// Wait for a release / poll interval and retry.
    Wait,
    /// Provider failed; gate has fallen back to fraction-only and admitted.
    AdmittedFallback,
}

impl Inner {
    fn try_admit(&self, weight: u64) -> AttemptOutcome {
        let mut state = self.state.lock().expect("weighted gate poisoned");

        if !state.scheduler_active {
            state.committed_bytes = state.committed_bytes.saturating_add(weight);
            state.active_permits += 1;
            return AttemptOutcome::AdmittedFallback;
        }

        // Refresh stats if stale.
        let stats = match self.fresh_stats(&mut state) {
            Ok(s) => s,
            Err(_) => {
                // Provider failure was already logged inside fresh_stats.
                state.committed_bytes = state.committed_bytes.saturating_add(weight);
                state.active_permits += 1;
                return AttemptOutcome::AdmittedFallback;
            }
        };

        let used_fraction = stats.used_fraction();
        if used_fraction >= self.config.base.max_ram_fraction {
            log_throttle(
                &mut state,
                "used-RAM fraction",
                used_fraction,
                self.config.base.max_ram_fraction,
            );
            return AttemptOutcome::Wait;
        }

        // Page-cache pressure: even if `MemAvailable` looks healthy, a
        // ballooning `Buffers+Cached` is the canary for an I/O-induced
        // readahead/swap-thrash death spiral. Disabled by default
        // (`max_page_cache_fraction == 1.0`) because the recommended
        // containment is a cgroup `MemoryHigh`, which the kernel enforces
        // without our help.
        if stats.total_bytes > 0 && self.config.max_page_cache_fraction < 1.0 {
            let cache_fraction = stats.page_cache_bytes as f64 / stats.total_bytes as f64;
            if cache_fraction >= self.config.max_page_cache_fraction {
                log_throttle(
                    &mut state,
                    "kernel page-cache",
                    cache_fraction,
                    self.config.max_page_cache_fraction,
                );
                return AttemptOutcome::Wait;
            }
        }

        // Made it past the throttle checks — clear the edge so the next
        // throttle is reported, and emit a single resume line.
        if state.throttled {
            tracing::info!(
                used_fraction,
                committed_bytes = state.committed_bytes,
                "weighted gate: resuming admissions"
            );
            state.throttled = false;
        }

        let oversized = weight > self.config.max_single_weight_bytes;
        if oversized {
            // Oversized tasks block on having the gate completely empty so
            // they are guaranteed maximum runway.
            if state.active_permits > 0 {
                return AttemptOutcome::Wait;
            }
            state.committed_bytes = state.committed_bytes.saturating_add(weight);
            state.active_permits += 1;
            state.oversized_in_flight = true;
            return AttemptOutcome::Admitted;
        }

        if state.oversized_in_flight {
            return AttemptOutcome::Wait;
        }

        let safety = self.config.safety_reserve_bytes;
        let budget = stats
            .available_bytes
            .saturating_sub(safety)
            .saturating_sub(state.committed_bytes);

        if budget >= weight {
            state.committed_bytes = state.committed_bytes.saturating_add(weight);
            state.active_permits += 1;
            AttemptOutcome::Admitted
        } else {
            AttemptOutcome::Wait
        }
    }

    fn release(&self, weight: u64, was_oversized: bool) {
        let mut state = self.state.lock().expect("weighted gate poisoned");
        state.committed_bytes = state.committed_bytes.saturating_sub(weight);
        if state.active_permits > 0 {
            state.active_permits -= 1;
        }
        if was_oversized {
            state.oversized_in_flight = false;
        }
        // Invalidate cached stats so the next admit-attempt re-probes; freed
        // memory shows up in `MemAvailable` before the kernel updates async
        // counters, but `MemAvailable` reflects the freed budget within a
        // few milliseconds.
        state.cached_at = None;
        #[cfg(feature = "sync")]
        self.condvar.notify_all();
        #[cfg(feature = "async")]
        self.notify.notify_waiters();
    }

    fn fresh_stats(&self, state: &mut GateState) -> Result<MemoryStats, ProviderError> {
        if let (Some(cached), Some(when)) = (state.cached_stats, state.cached_at)
            && when.elapsed() < self.config.stats_max_age
        {
            return Ok(cached);
        }
        match self.provider.stats() {
            Ok(stats) => {
                state.cached_stats = Some(stats);
                state.cached_at = Some(Instant::now());
                Ok(stats)
            }
            Err(e) => {
                if !state.failure_logged {
                    tracing::warn!(error = %e, "weighted memory provider failed; falling back to thread-cap-only");
                    state.failure_logged = true;
                }
                state.scheduler_active = false;
                Err(e)
            }
        }
    }
}

fn log_throttle(state: &mut GateState, kind: &'static str, current: f64, threshold: f64) {
    if state.throttled {
        return;
    }
    state.throttled = true;
    tracing::warn!(
        kind,
        current_fraction = current,
        threshold,
        active_permits = state.active_permits,
        committed_bytes = state.committed_bytes,
        "weighted gate: throttling"
    );
}

/// Synchronous weighted admission gate.
#[cfg(feature = "sync")]
#[derive(Clone)]
pub struct SyncWeightedAdmissionGate {
    inner: Arc<Inner>,
}

#[cfg(feature = "sync")]
impl SyncWeightedAdmissionGate {
    /// Build a gate using [`WeightedConfig::validated_default`] and
    /// [`providers::default_provider`].
    ///
    /// [`providers::default_provider`]: crate::providers::default_provider
    #[must_use]
    pub fn new_default() -> Self {
        Self::new(
            WeightedConfig::validated_default(),
            crate::providers::default_provider(),
        )
    }

    /// Build a gate from a validated config and provider.
    #[must_use]
    pub fn new(config: WeightedConfig, provider: SharedMemoryProvider) -> Self {
        let scheduler_active = config.base.memory_scheduler_enabled;
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(GateState {
                    committed_bytes: 0,
                    active_permits: 0,
                    oversized_in_flight: false,
                    cached_stats: None,
                    cached_at: None,
                    scheduler_active,
                    failure_logged: false,
                    throttled: false,
                }),
                condvar: Condvar::new(),
                #[cfg(feature = "async")]
                notify: Notify::new(),
                provider,
                config,
            }),
        }
    }

    /// Block until `weight_bytes` of budget can be committed, then return a permit.
    pub fn acquire(&self, weight_bytes: u64) -> WeightedPermit {
        loop {
            match self.inner.try_admit(weight_bytes) {
                AttemptOutcome::Admitted => {
                    let oversized = weight_bytes > self.inner.config.max_single_weight_bytes;
                    return WeightedPermit::new(self.inner.clone(), weight_bytes, oversized);
                }
                AttemptOutcome::AdmittedFallback => {
                    return WeightedPermit::new(self.inner.clone(), weight_bytes, false);
                }
                AttemptOutcome::Wait => {
                    let state = self.inner.state.lock().expect("weighted gate poisoned");
                    let _ = self
                        .inner
                        .condvar
                        .wait_timeout(state, self.inner.config.base.poll_interval)
                        .expect("weighted gate poisoned");
                }
            }
        }
    }

    /// Currently committed weight.
    pub fn committed_bytes(&self) -> u64 {
        self.inner
            .state
            .lock()
            .expect("weighted gate poisoned")
            .committed_bytes
    }

    /// Number of weighted permits currently in flight.
    pub fn active_permits(&self) -> usize {
        self.inner
            .state
            .lock()
            .expect("weighted gate poisoned")
            .active_permits
    }

    /// Whether memory-aware scheduling is currently active.
    pub fn memory_scheduler_active(&self) -> bool {
        self.inner
            .state
            .lock()
            .expect("weighted gate poisoned")
            .scheduler_active
    }
}

/// Asynchronous weighted admission gate.
#[cfg(feature = "async")]
#[derive(Clone)]
pub struct AsyncWeightedAdmissionGate {
    inner: Arc<Inner>,
}

#[cfg(feature = "async")]
impl AsyncWeightedAdmissionGate {
    /// Build a gate using [`WeightedConfig::validated_default`] and
    /// [`providers::default_provider`].
    ///
    /// [`providers::default_provider`]: crate::providers::default_provider
    #[must_use]
    pub fn new_default() -> Self {
        Self::new(
            WeightedConfig::validated_default(),
            crate::providers::default_provider(),
        )
    }

    /// Build a gate from a validated config and provider.
    #[must_use]
    pub fn new(config: WeightedConfig, provider: SharedMemoryProvider) -> Self {
        let scheduler_active = config.base.memory_scheduler_enabled;
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(GateState {
                    committed_bytes: 0,
                    active_permits: 0,
                    oversized_in_flight: false,
                    cached_stats: None,
                    cached_at: None,
                    scheduler_active,
                    failure_logged: false,
                    throttled: false,
                }),
                #[cfg(feature = "sync")]
                condvar: Condvar::new(),
                notify: Notify::new(),
                provider,
                config,
            }),
        }
    }

    /// Yield until `weight_bytes` of budget can be committed, then return a permit.
    pub async fn acquire(&self, weight_bytes: u64) -> WeightedPermit {
        loop {
            match self.inner.try_admit(weight_bytes) {
                AttemptOutcome::Admitted => {
                    let oversized = weight_bytes > self.inner.config.max_single_weight_bytes;
                    return WeightedPermit::new(self.inner.clone(), weight_bytes, oversized);
                }
                AttemptOutcome::AdmittedFallback => {
                    return WeightedPermit::new(self.inner.clone(), weight_bytes, false);
                }
                AttemptOutcome::Wait => {
                    let notified = self.inner.notify.notified();
                    tokio::select! {
                        () = notified => {}
                        () = tokio::time::sleep(self.inner.config.base.poll_interval) => {}
                    }
                }
            }
        }
    }

    /// Currently committed weight.
    pub fn committed_bytes(&self) -> u64 {
        self.inner
            .state
            .lock()
            .expect("weighted gate poisoned")
            .committed_bytes
    }

    /// Number of weighted permits currently in flight.
    pub fn active_permits(&self) -> usize {
        self.inner
            .state
            .lock()
            .expect("weighted gate poisoned")
            .active_permits
    }

    /// Whether memory-aware scheduling is currently active.
    pub fn memory_scheduler_active(&self) -> bool {
        self.inner
            .state
            .lock()
            .expect("weighted gate poisoned")
            .scheduler_active
    }
}

/// RAII permit returned by both sync and async weighted gates.
pub struct WeightedPermit {
    inner: Option<Arc<Inner>>,
    weight: u64,
    oversized: bool,
}

impl WeightedPermit {
    fn new(inner: Arc<Inner>, weight: u64, oversized: bool) -> Self {
        Self {
            inner: Some(inner),
            weight,
            oversized,
        }
    }

    /// The byte weight this permit committed.
    #[must_use]
    pub fn weight(&self) -> u64 {
        self.weight
    }
}

impl Drop for WeightedPermit {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.release(self.weight, self.oversized);
        }
    }
}

/// Ergonomic alias for the synchronous weighted gate.
#[cfg(feature = "sync")]
pub type SyncAdmissionGate = SyncWeightedAdmissionGate;

/// Ergonomic alias for the asynchronous weighted gate.
#[cfg(feature = "async")]
pub type AsyncAdmissionGate = AsyncWeightedAdmissionGate;

/// Ergonomic alias for the weighted permit type.
pub type Permit = WeightedPermit;

#[cfg(test)]
mod tests {
    #[cfg(feature = "async")]
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::providers::FixedProvider;

    #[cfg(feature = "async")]
    #[derive(Debug)]
    struct StatsProvider {
        stats: MemoryStats,
        fail_stats: bool,
    }

    #[cfg(feature = "async")]
    impl crate::provider::MemoryProvider for StatsProvider {
        fn used_fraction(&self) -> Result<f64, ProviderError> {
            Ok(self.stats.used_fraction())
        }

        fn stats(&self) -> Result<MemoryStats, ProviderError> {
            if self.fail_stats {
                Err(ProviderError::new("stats unavailable"))
            } else {
                Ok(self.stats)
            }
        }
    }

    #[cfg(feature = "async")]
    fn stats_provider(available_bytes: u64) -> SharedMemoryProvider {
        Arc::new(StatsProvider {
            stats: MemoryStats {
                total_bytes: 1024,
                available_bytes,
                page_cache_bytes: 0,
            },
            fail_stats: false,
        })
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn admits_under_budget() {
        let cfg = WeightedConfig {
            base: Config::default().validate().unwrap(),
            safety_reserve_bytes: 0,
            max_single_weight_bytes: u64::MAX,
            ..WeightedConfig::default()
        };
        let gate = AsyncWeightedAdmissionGate::new(cfg, FixedProvider::shared(0.10));
        let p = gate.acquire(1024).await;
        assert_eq!(p.weight(), 1024);
        assert_eq!(gate.committed_bytes(), 1024);
        assert_eq!(gate.active_permits(), 1);
        assert!(gate.memory_scheduler_active());
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn release_decrements_committed_bytes() {
        let cfg = WeightedConfig {
            base: Config::default().validate().unwrap(),
            safety_reserve_bytes: 0,
            max_single_weight_bytes: u64::MAX,
            ..WeightedConfig::default()
        };
        let gate = AsyncWeightedAdmissionGate::new(cfg, FixedProvider::shared(0.10));
        {
            let _p = gate.acquire(2048).await;
            assert_eq!(gate.committed_bytes(), 2048);
        }
        assert_eq!(gate.committed_bytes(), 0);
        assert_eq!(gate.active_permits(), 0);
    }

    #[cfg(feature = "sync")]
    #[test]
    fn sync_gate_admits_under_budget() {
        let cfg = WeightedConfig {
            base: Config::default().validate().unwrap(),
            safety_reserve_bytes: 0,
            max_single_weight_bytes: u64::MAX,
            ..WeightedConfig::default()
        };
        let gate = SyncWeightedAdmissionGate::new(cfg, FixedProvider::shared(0.10));
        let p = gate.acquire(1024);
        assert_eq!(p.weight(), 1024);
        assert_eq!(gate.active_permits(), 1);
    }

    #[test]
    fn validates_weighted_config() {
        assert!(WeightedConfig::default().validate().is_ok());

        let bad_cache = WeightedConfig {
            max_page_cache_fraction: f64::NAN,
            ..WeightedConfig::default()
        };
        assert!(matches!(
            bad_cache.validate(),
            Err(WeightedConfigError::MaxPageCacheFractionOutOfRange(_))
        ));

        let bad_age = WeightedConfig {
            stats_max_age: Duration::ZERO,
            ..WeightedConfig::default()
        };
        assert!(matches!(
            bad_age.validate(),
            Err(WeightedConfigError::StatsMaxAgeZero)
        ));

        let bad_base = WeightedConfig {
            base: Config {
                max_ram_fraction: 0.0,
                ..Config::default()
            },
            ..WeightedConfig::default()
        };
        assert!(matches!(
            bad_base.validate(),
            Err(WeightedConfigError::Base(_))
        ));
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_gate_blocks_until_budget_released() {
        let cfg = WeightedConfig {
            base: Config {
                poll_interval: Duration::from_millis(10),
                ..Config::default()
            }
            .validate()
            .unwrap(),
            safety_reserve_bytes: 0,
            max_single_weight_bytes: u64::MAX,
            stats_max_age: Duration::from_millis(1),
            ..WeightedConfig::default()
        };
        let gate = AsyncWeightedAdmissionGate::new(cfg, stats_provider(700));
        let first = gate.acquire(600).await;

        let waiter_gate = gate.clone();
        let waiter = tokio::spawn(async move { waiter_gate.acquire(200).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());

        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("weighted waiter should resume")
            .expect("weighted waiter task should not panic");
        assert_eq!(second.weight(), 200);
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn oversized_permit_runs_alone() {
        let cfg = WeightedConfig {
            base: Config {
                poll_interval: Duration::from_millis(10),
                ..Config::default()
            }
            .validate()
            .unwrap(),
            safety_reserve_bytes: 0,
            max_single_weight_bytes: 128,
            stats_max_age: Duration::from_millis(1),
            ..WeightedConfig::default()
        };
        let gate = AsyncWeightedAdmissionGate::new(cfg, stats_provider(1024));
        let small = gate.acquire(64).await;

        let waiter_gate = gate.clone();
        let waiter_started = Arc::new(AtomicUsize::new(0));
        let waiter_started_task = Arc::clone(&waiter_started);
        let waiter = tokio::spawn(async move {
            waiter_started_task.store(1, Ordering::SeqCst);
            waiter_gate.acquire(256).await
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(waiter_started.load(Ordering::SeqCst), 1);
        assert!(!waiter.is_finished());

        drop(small);
        let oversized = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("oversized waiter should resume")
            .expect("oversized waiter task should not panic");
        assert_eq!(oversized.weight(), 256);
        assert_eq!(gate.active_permits(), 1);
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn provider_failure_falls_back_and_admits() {
        let cfg = WeightedConfig {
            safety_reserve_bytes: 0,
            ..WeightedConfig::default()
        };
        let provider = Arc::new(StatsProvider {
            stats: MemoryStats::default(),
            fail_stats: true,
        });
        let gate = AsyncWeightedAdmissionGate::new(cfg, provider);
        let permit = gate.acquire(2048).await;
        assert_eq!(permit.weight(), 2048);
        assert!(!gate.memory_scheduler_active());
        assert_eq!(gate.committed_bytes(), 2048);
    }

    #[cfg(feature = "sync")]
    #[test]
    fn sync_new_default_constructs() {
        let _gate = SyncWeightedAdmissionGate::new_default();
    }

    #[cfg(feature = "async")]
    #[test]
    fn async_new_default_constructs() {
        let _gate = AsyncWeightedAdmissionGate::new_default();
    }
}
