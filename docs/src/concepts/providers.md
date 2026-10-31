# Providers

A memory provider returns current memory usage as a fraction in the inclusive range `[0.0, 1.0]`.

The default provider selection is:

1. Linux cgroup-v2 accounting when the current process is in a cgroup-v2 hierarchy.
2. Linux `/proc/meminfo`.
3. `sysinfo` on non-Linux targets when the `sysinfo` feature is enabled.

The cgroup-v2 provider reports availability as the limit (`memory.high`, else `memory.max`) minus the working set, `memory.current - inactive_file` (the kubelet/cAdvisor definition). `memory.current` includes page cache, so subtracting only inactive file pages keeps availability from collapsing after a large build while still counting active, dirty, and writeback pages as in use. If `memory.stat` is unreadable it falls back to `memory.current`.

Custom providers can implement `MemoryProvider` for other memory sources such as NUMA-specific accounting, container runtimes, or test fixtures.
