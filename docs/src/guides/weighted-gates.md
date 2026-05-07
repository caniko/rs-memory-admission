# Weighted Gates

Weighted gates reserve the caller's declared byte weight until the returned permit is dropped.

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

Oversized jobs wait until the gate is otherwise empty, then run alone so they do not deadlock the system.
