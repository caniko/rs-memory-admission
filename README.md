# memory-admission

<!-- simit:badges:start -->

[![CI](https://img.shields.io/badge/CI-drift-2088ff)](.forgejo/workflows/ci.yaml) [![Nix](https://img.shields.io/badge/Nix-managed-5277c3)](flake.nix) [![docs](https://img.shields.io/badge/docs-enabled-6f42c1)](docs) [![crates.io](https://img.shields.io/badge/crates.io-ready-f46623)](https://crates.io/crates/memory-admission)

<!-- simit:badges:end -->

`memory-admission` is a Rust admission gate for parallel work that should slow
down when the host or cgroup is under memory pressure. It exposes synchronous
and Tokio-friendly gates, plus weighted gates for jobs that can estimate their
memory cost in bytes.

[![docs.rs](https://docs.rs/memory-admission/badge.svg)](https://docs.rs/memory-admission)
[![crates.io](https://img.shields.io/crates/v/memory-admission.svg)](https://crates.io/crates/memory-admission)

Use the unweighted gates when each task has roughly the same memory profile. Use
weighted gates when a pipeline mixes cheap jobs with large allocations,
subprocesses, archive extraction, media processing, or other work where a
worker count alone is not a useful safety limit.

## Installation

```toml
[dependencies]
memory-admission = "0.1.7"
```

Disable default features if you only need one gate style or want to avoid the
cross-platform `sysinfo` provider:

```toml
[dependencies]
memory-admission = { version = "0.1.7", default-features = false, features = ["async"] }
```

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

## Links

- API docs: <https://docs.rs/memory-admission>
- Source: <https://github.com/caniko/rs-memory-admission>
- Project docs: <https://caniko.codeberg.page/rs-memory-admission/>

## Nix

The flake builds the crate with crane through
`git+https://github.com/caniko/harbor-rs.git`.

```sh
nix build
nix flake check
nix develop
```

## Release Validation

```sh
nix flake check --keep-going --print-build-logs
nix develop -c cargo package --list
nix develop -c cargo publish --dry-run
```

## License

Licensed under either of:

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.
