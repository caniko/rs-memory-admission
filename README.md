# memory-admission

`memory-admission` is a Rust admission gate for parallel work that should slow
down when the host or cgroup is under memory pressure. It exposes synchronous
and Tokio-friendly gates, plus weighted gates for jobs that can estimate their
memory cost in bytes.

Use the unweighted gates when each task has roughly the same memory profile. Use
weighted gates when a pipeline mixes cheap jobs with large allocations,
subprocesses, archive extraction, media processing, or other work where a
worker count alone is not a useful safety limit.

## Providers

The default provider is selected with `providers::default_provider()`:

1. Linux cgroup-v2 accounting when the current process is in a cgroup-v2
   hierarchy.
2. Linux `/proc/meminfo`.
3. `sysinfo` on non-Linux targets when the `sysinfo` feature is enabled.

If a provider fails during initialization or at runtime, gates log once and
fall back to thread-cap-only admission instead of failing user work.

## Synchronous gate

```rust
use memory_admission::sync::AdmissionGate;

let gate = AdmissionGate::new_default();

let permit = gate.acquire();
// Run memory-sensitive work while the permit is held.
drop(permit);
```

## Async gate

```rust
use memory_admission::r#async::AdmissionGate;

let gate = AdmissionGate::new_default();

let permit = gate.acquire().await;
// Run memory-sensitive async work while the permit is held.
drop(permit);
```

## Weighted gates

```rust
use memory_admission::weighted::{AsyncAdmissionGate, WeightedConfig};

let config = WeightedConfig::validated_default();
let gate = AsyncAdmissionGate::new(
    config,
    memory_admission::providers::default_provider(),
);

let permit = gate.acquire(512 * 1024 * 1024).await;
// Run work expected to commit about 512 MiB.
drop(permit);
```

Weighted gates reserve the caller's declared byte weight until the returned
permit is dropped. Oversized jobs wait until the gate is otherwise empty, then
run alone so they do not deadlock the system.

## Nix

The flake builds the crate with crane through
`git+https://codeberg.org/caniko/rs-harbor.git`.

```sh
nix build
nix flake check
nix develop
```
