# APFS multi-process writer stress evidence — 2026-10-03

## Subject

- Commit: `7e1213b5ecdf3c2717937eabb0e62603de7165a8`
- Host: Apple arm64, internal solid-state storage
- OS: macOS 26.6.2 (25G83)
- Filesystem: APFS
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`

## Command

```text
WSLDB_RUN_MULTIPROCESS_STRESS=1 \
  cargo test --release --test multiprocess_stress -- \
  --nocapture --test-threads=1
```

The direct and durable-buffer cases each started 12 independent processes from
the same committed head. Every process kept one database handle, retried only a
typed stale-head conflict after refreshing that handle, and completed 500
durable transactions. Each transaction atomically inserted a marker and a
257-byte deterministic payload under unique keys. After every successful
transaction, the worker also staged and dropped one additional mutation to
exercise abort-on-drop.

The parent required actual conflicts, checked the final transaction ID and full
structural verification, and compared the complete ordered contents with an
independently constructed map. The durable-buffer case additionally consolidated
the WAL, reopened the database, verified it again, and repeated the full logical
comparison.

## Result

| Mode | Durable transactions | Final entries | Rejected stale heads | Elapsed |
| --- | ---: | ---: | ---: | ---: |
| Durable buffer | 6,000 | 12,000 | 19,138 | 393.342 s |
| Direct page tree | 6,000 | 12,000 | 19,993 | 67.982 s |

- Status: passed
- Total wall time, including both modes: 461.37 seconds
- Lost or duplicated acknowledged transactions: zero
- Visible dropped mutations: zero
- Panics, hangs, corruption, or unexpected errors: zero

This is correctness and liveness evidence on one APFS host, not a throughput
claim. In particular, the durable-buffer result exposes expensive full WAL
recovery after each cross-process conflict; that path needs incremental refresh
or contention backoff before it can be described as write-scalable. Linux,
Windows, other filesystems, process termination mid-commit, and real power loss
remain separate release gates.

The subsequent incremental-refresh implementation and an exact rerun of this
workload are documented [here](2026-10-03-incremental-buffer-refresh.md).
