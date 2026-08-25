# memory-admission

`memory-admission` is a Rust admission gate for parallel work that should slow down when the host or cgroup is under memory pressure.

It provides synchronous gates, Tokio-friendly gates, and weighted gates for jobs that can estimate their memory cost in bytes. Use it when a worker count alone is not enough to protect a machine from swap thrashing or out-of-memory pressure.

Source is hosted at [GitHub](https://github.com/caniko/rs-memory-admission).
