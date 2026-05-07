# Providers

A memory provider returns current memory usage as a fraction in the inclusive range `[0.0, 1.0]`.

The default provider selection is:

1. Linux cgroup-v2 accounting when the current process is in a cgroup-v2 hierarchy.
2. Linux `/proc/meminfo`.
3. `sysinfo` on non-Linux targets when the `sysinfo` feature is enabled.

Custom providers can implement `MemoryProvider` for other memory sources such as NUMA-specific accounting, container runtimes, or test fixtures.
