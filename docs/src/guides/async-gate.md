# Async Gate

The async gate is backed by `tokio::sync::Notify` and is intended for Tokio task fanout and stream pipelines.

```rust
use memory_admission::r#async::AdmissionGate;

let gate = AdmissionGate::new_default();
let permit = gate.acquire().await;

// Do memory-sensitive async work.

drop(permit);
```

Hold the permit for the lifetime of the operation that contributes to memory pressure.
