# Lazy-index W2 comparison with RocksDB 11.8.1

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is a diagnostic for one workload and platform, not a general superiority
claim.

## Reproducibility

- WSLDB commit: `ab539b9a372febc6281ef9c36ffa32e3e657987e`.
- WSLDB worktree: clean.
- RocksDB source: official `v11.8.1` archive, SHA-256
  `618d9726a7cb1cf4ce034f4cdca49de98aa64867dda06b91371a791ae8921aff`.
- Fresh local `db_bench` SHA-256:
  `a6104a34cc935d696db3a76a535e4edccaeedef5d7e0475a883656a63d152ec3`.
- Dataset: 1,000,000 unique 16-byte keys and 100-byte values.
- Batch: 10,000 operations; one warm-up and five alternating measured fresh
  processes per engine.
- Durability: synchronous commits, WAL enabled for both engines.
- Compression: disabled.
- Cache budget: 256 MiB; WSLDB durable-buffer threshold: 64 MiB.

The RocksDB binary was rebuilt from the recorded source archive with release
optimization, portable instructions, RTTI, and benchmark tools enabled. The
runner, commands, complete stdout/stderr, process-resource reports, sizes, and
host manifest are retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-lazy-index-fresh-rocksdb-w2`.

## Result

| Metric | WSLDB | RocksDB | Outcome |
| --- | ---: | ---: | ---: |
| Median active throughput | 529,671.923 ops/s | 558,157 ops/s | RocksDB +5.38% |
| Median maximum RSS | 128,237,568 B | 114,704,384 B | WSLDB +11.80% |
| Median logical storage | 123,449,976 B | 121,951,144 B | WSLDB +1.23% |
| Median whole-process time | 2.76 s | 7.07 s | WSLDB -60.96% |
| Median maximum durable-batch latency | 256.278 ms | 40.749 ms | RocksDB 6.29x lower |

The active interval is the relevant result for the existing W2 gate: WSLDB is
close but has not caught RocksDB. The whole-process number is reported, not
claimed as a win. RocksDB's process explicitly flushes and waits for compaction
after its active interval; WSLDB finishes with a fully durable, recoverable WAL
tail and does not materialize that tail into the main file. Those are different
postconditions.

WSLDB's long maximum commit is its synchronous threshold checkpoint. Improving
that tail without hiding work or weakening durability remains a release task.
The two engines also need cold-cache and supported-platform measurements before
any broader performance statement.
