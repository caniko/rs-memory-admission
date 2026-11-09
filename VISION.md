# memory-admission vision

`memory-admission` is a Rust admission gate for parallel work that should slow
down when the host or its cgroup is under memory pressure. It offers a
synchronous gate, a Tokio-friendly gate, and weighted gates for jobs that can
estimate their own memory cost in bytes. It is a library, not a runtime or a
scheduler.

## Problem

A worker count is a poor safety limit. A fixed pool of eight workers is fine
when each job allocates a little memory and fatal when the same pipeline mixes
small jobs with archive extraction, media processing, or large subprocesses. The
pressure that matters is memory on the machine and in the cgroup, which changes
while the pipeline runs.

## Principles

- Gate on observed memory pressure, not on a static number of workers. A permit
  is held while memory-sensitive work runs and released when it finishes.
- Providers are layered: Linux cgroup-v2 accounting when the process is in a
  cgroup-v2 hierarchy, then Linux `/proc/meminfo`, then `sysinfo` on other
  targets when that feature is enabled.
- Provider failure must not fail user work. If the provider cannot initialize or
  later fails, the gate logs once and falls back to thread-cap-only admission.
- Throttling uses hysteresis: resume below `max_ram_fraction - resume_hysteresis`
  so releasing one task cannot immediately admit another and re-cross the limit.
- Weighted gates reserve the caller's declared byte weight. A job larger than
  the gate waits until the gate is otherwise empty and then runs alone, so
  oversized work cannot deadlock the system.
- Gate style is a feature choice (`async`, `sysinfo`, weighted surface) so a
  consumer that needs one gate does not pull the other provider stack.

## Scope

- Synchronous, async, and weighted admission gates.
- Provider selection and fallback for Linux cgroups, `/proc/meminfo`, and
  `sysinfo`.
- Configuration for pressure thresholds, hysteresis, and weight accounting.

## Non-goals

- It does not manage cgroups, set memory limits, or kill processes.
- It does not replace a job scheduler or executor; it only decides whether a
  caller may hold a permit now.
- It does not measure actual allocation. Weighted gates trust the caller's
  declared weight.

## Direction

The 0.1.7 release was release-tooling repair: rustdoc links, all-feature clippy
coverage, and dependency audit/deny policy in the release path. Commit history
since then is build and CI maintenance. No roadmap is checked in.

## Place in the fleet

No canix runtime consumer was found. Canix records the project only in the atlas
`simit-updater` registry.
