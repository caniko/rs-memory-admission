# Hysteresis

The gate enters a throttled state when memory usage exceeds `Config::max_ram_fraction`.

It resumes only after usage drops below `max_ram_fraction - resume_hysteresis`. This prevents a cycle where releasing one task admits another task immediately and pushes memory usage back over the limit.
