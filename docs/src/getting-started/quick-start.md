# Quick Start

Use an admission gate around the work that creates memory pressure.

```rust
use memory_admission::sync::AdmissionGate;

let gate = AdmissionGate::new_default();

let permit = gate.acquire();
// Run memory-sensitive work while the permit is held.
drop(permit);
```

For async workloads, use the Tokio-friendly gate:

```rust
use memory_admission::r#async::AdmissionGate;

let gate = AdmissionGate::new_default();

let permit = gate.acquire().await;
// Run memory-sensitive async work while the permit is held.
drop(permit);
```
