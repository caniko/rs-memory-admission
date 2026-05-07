# Synchronous Gate

The synchronous gate is backed by `std::sync::Condvar` and is intended for thread-pool-based work.

```rust
use memory_admission::sync::AdmissionGate;

let gate = AdmissionGate::new_default();
let permit = gate.acquire();

// Do memory-sensitive work.

drop(permit);
```

Dropping the permit releases the admission slot and wakes waiting callers when appropriate.
