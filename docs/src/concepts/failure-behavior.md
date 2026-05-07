# Failure Behavior

Provider failures should not fail user work.

If the memory provider fails during initialization, the gate falls back to thread-cap-only admission and logs the reason. Later provider failures during acquisition also disable memory throttling once and allow callers to continue.

This behavior is useful in sandboxes or restricted containers where `/proc/meminfo`, cgroup files, or platform APIs can be unavailable.
