# Incremental durable-buffer refresh evidence — 2026-10-03

## Question

Can a stale buffered handle replay only records appended after its previously
validated WAL boundary, without weakening snapshot isolation, torn-tail
recovery, or generation-change detection?

## Subjects

- Baseline: `7e1213b5ecdf3c2717937eabb0e62603de7165a8`
- Candidate: `05d3c5cfe4f4f4999ccb866b7896ca26e042ca12`
- Host: Apple arm64, internal solid-state storage
- OS: macOS 26.6.2 (25G83)
- Filesystem: APFS
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`

## Protocol

Both subjects ran the same frozen command and workload:

```text
WSLDB_RUN_MULTIPROCESS_STRESS=1 \
  cargo test --release --test multiprocess_stress -- \
  --nocapture --test-threads=1
```

Each mode used 12 independent processes and 500 durable transactions per
process. Every transaction inserted two deterministic unique entries. Workers
started from one head, refreshed and retried only typed stale-head conflicts,
and staged one deliberately aborted mutation after every acknowledged commit.
The parent performed structural verification and an exact full ordered-model
comparison. The buffered case also flushed, reopened, and repeated verification
and comparison.

The candidate retains the full reopen path whenever the page-tree checkpoint or
WAL inode changes. With the same generations, it starts at the handle's prior
validated WAL end, verifies record checksums and the transaction chain, applies
only the complete tail, and truncates an incomplete physical suffix before
publishing the refreshed head.

## Result

| Mode | Baseline | Candidate | Change | Baseline conflicts | Candidate conflicts |
| --- | ---: | ---: | ---: | ---: | ---: |
| Durable buffer | 393.342 s | 36.479 s | 10.78x faster | 19,138 | 19,404 |
| Direct page tree control | 67.982 s | 68.029 s | 0.1% slower | 19,993 | 17,357 |

- Candidate status: passed
- Candidate total test time: 104.56 seconds
- Final transaction ID per mode: 6,000
- Final entries per mode: 12,000
- Lost or duplicated acknowledged transactions: zero
- Visible aborted or torn-tail mutations: zero
- Panics, hangs, corruption, or unexpected errors: zero
- Full repository tests, Clippy with warnings denied, check, formatting, and
  diff validation: passed

The buffered elapsed reduction is 90.7% for this high-conflict stress workload.
This is direct A/B evidence for incremental refresh, not a general database
throughput claim and not a comparison with RocksDB. It is a single-host APFS
result; the supported-platform and real-power-loss matrices remain open.
