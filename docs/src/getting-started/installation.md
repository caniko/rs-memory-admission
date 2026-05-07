# Installation

Add the crate to your Rust project:

```sh
cargo add memory-admission
```

The default feature set enables the synchronous API, the async API, and the cross-platform `sysinfo` provider.

To choose a smaller API surface, disable default features and opt in:

```toml
[dependencies]
memory-admission = { version = "0.1.0", default-features = false, features = ["async"] }
```
