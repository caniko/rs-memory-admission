//! Memory-aware admission gate for parallel work pipelines.
//!
//! Pipelines that fan out a large number of memory-hungry tasks (archive
//! extraction, video decode, scientific batch jobs) can drive the host into
//! swap thrashing or OOM territory long before they exhaust thread pools.
//! `memory-admission` provides an admission gate that pauses *new* task
//! admissions when host RAM usage crosses a configurable threshold, then
//! resumes once usage falls back below a hysteresis band.
//!
//! Two flavours are provided:
//!
//! - `sync::AdmissionGate` — backed by `std::sync::Condvar`. Designed to wrap
//!   Rayon parallel iteration so blocking acquisition costs zero scheduler
//!   wakeups.
//! - `async::AdmissionGate` — backed by `tokio::sync::Notify`. Designed for
//!   `futures::stream::buffer_unordered` consumers and other Tokio task fanout.
//!
//! Both gates share the same configuration and the same memory provider
//! abstraction, so a single project can mix the two.
//!
//! ## Memory providers
//!
//! A [`MemoryProvider`] returns the current "used" fraction of host RAM as a
//! `f64` in the inclusive range `[0.0, 1.0]`. The crate ships:
//!
//! - [`providers::ProcMeminfoProvider`] — Linux-only, reads `/proc/meminfo`.
//!   Cheap and always-current.
//! - [`providers::SysinfoProvider`] — cross-platform, gated by the `sysinfo`
//!   feature.
//! - [`providers::FixedProvider`] — for tests.
//!
//! A custom provider can be supplied for any other source (cgroup memory peak,
//! NUMA-node specific stats, mocks).
//! Use [`providers::default_provider`] for the crate's platform-aware default.
//!
//! ## Hysteresis
//!
//! When usage exceeds [`Config::max_ram_fraction`] the gate enters the
//! throttled state. It only leaves throttled once usage has dropped to
//! `(max_ram_fraction - resume_hysteresis)`. This prevents oscillation when
//! task release frees just enough memory to admit another task that would
//! immediately push usage back over the line.
//!
//! ## Provider failure
//!
//! If the memory provider returns an error during initialization the gate
//! falls back to thread-cap-only operation (no throttling) and logs the
//! reason. Subsequent provider failures during `acquire` calls also disable
//! the scheduler exactly once and proceed unthrottled. This keeps callers
//! resilient when running in a sandbox that hides `/proc/meminfo` or revokes
//! `sysinfo` permissions mid-run.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod config;
pub mod provider;
pub mod providers;
pub mod weighted;

#[cfg(feature = "sync")]
pub mod sync;

#[cfg(feature = "async")]
pub mod r#async;

pub use config::{Config, ConfigError};
pub use provider::{MemoryProvider, MemoryStats, ProviderError};
