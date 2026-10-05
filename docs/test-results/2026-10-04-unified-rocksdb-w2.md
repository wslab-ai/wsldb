# Unified write path W2 comparison with RocksDB 11.8.1

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is a diagnostic for one workload and platform, not a general superiority
claim.

## Reproducibility

- WSLDB commit: `55153a69c5bc7d8f338f246e67383db88c9412aa`.
- WSLDB benchmark binary SHA-256:
  `5aea79e6ba6e0a88e5653618c3dee4c99062914dd49d46c3d09495a5a61ca9af`.
- RocksDB source: official `v11.8.1` archive, SHA-256
  `618d9726a7cb1cf4ce034f4cdca49de98aa64867dda06b91371a791ae8921aff`.
- Fresh local `db_bench` SHA-256:
  `56cc46094e50ff4502a94f86f7203206642e702593d197b1f41d9c46ddabd667`.
- Dataset: 1,000,000 unique 16-byte keys and 100-byte values.
- Commit batch: 10,000 operations; one warm-up and five alternating measured
  fresh processes per engine.
- Durability: synchronous commits with WAL enabled for both engines.
- Compression: disabled. Cache budget: 256 MiB. WSLDB durable-buffer
  threshold: 64 MiB.

RocksDB was rebuilt locally in release mode with portable instructions, RTTI,
tests disabled, and benchmark tools enabled. Its active interval is
`filluniquerandom`; the process then explicitly flushes and waits for
compaction. WSLDB's active interval ends with a fully recoverable durable WAL
tail. Complete output, resource reports, sizes, hashes, build flags, and host
manifest are retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-unified-fresh-rocksdb-w2`.

## Result

| Metric | WSLDB | RocksDB | Outcome |
| --- | ---: | ---: | ---: |
| Median active throughput | 524,188.733 ops/s | 491,990 ops/s | WSLDB +6.55% |
| Median maximum RSS | 127,565,824 B | 118,898,688 B | WSLDB +7.29% |
| Median logical storage | 123,449,976 B | 121,950,657 B | WSLDB +1.23% |
| Median maximum durable-batch latency | 297.086 ms | 46.309 ms | RocksDB 6.42x lower |
| Median whole-process time | 2.77 s | 7.38 s | Different postconditions |

The active W2 throughput comparison now favors WSLDB on this exact session.
The whole-process number is not counted as a win because RocksDB performs an
explicit flush and compaction wait after its active interval. Conversely,
RocksDB retains decisive tail-latency, RSS, and storage advantages. Those gaps,
plus cold-cache and supported-platform coverage, still block a broad claim
that WSLDB is faster or better than RocksDB.
