+++
title = "memory-admission"

[extra]
tagline = "Memory-aware admission for Rust work pipelines"
subtitle = "Sync, async, and weighted gates that slow new work when host or cgroup memory pressure crosses a configurable threshold."

[[extra.features]]
title = "Synchronous gates"
body = "Condvar-backed admission for thread pools, Rayon-style fanout, and blocking pipelines."

[[extra.features]]
title = "Tokio-friendly gates"
body = "Async admission built on tokio::sync::Notify for stream and task fanout."

[[extra.features]]
title = "Weighted work"
body = "Reserve estimated bytes per job so large allocations and subprocesses can run without relying on worker count alone."

[[extra.features]]
title = "cgroup-v2 first"
body = "Prefer cgroup-v2 accounting when the process is memory bounded by the host."

[[extra.features]]
title = "/proc/meminfo provider"
body = "Use cheap Linux MemAvailable readings when cgroup accounting is not active."

[[extra.features]]
title = "sysinfo fallback"
body = "Use a cross-platform provider when the sysinfo feature is enabled."

[[extra.features]]
title = "Provider failure tolerance"
body = "Log provider failures once and fall back to thread-cap-only admission instead of failing user work."

[[extra.features]]
title = "Hysteresis"
body = "Resume work only after memory usage drops below the configured band to avoid oscillation."

[[extra.features]]
title = "No unsafe code"
body = "The crate forbids unsafe code at the crate root."
+++
