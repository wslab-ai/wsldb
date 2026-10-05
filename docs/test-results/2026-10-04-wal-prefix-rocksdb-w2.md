# WAL-prefix W2 comparison with RocksDB 11.8.1

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is evidence for one durable random-load profile, not a general claim that
WSLDB is faster or better than RocksDB.

## Reproducibility

- WSLDB commit: `e32ae7b338e69d75c5082beb85d9b1f321f8e19f`.
- WSLDB storage implementation: `4d3e59c`.
- WSLDB benchmark SHA-256:
  `584140f31ebd949eab3a0ac28673cabcdd04f3bfa080eee41c6d69d7cac34ed1`.
- RocksDB source: official tag `v11.8.1`, commit
  `abeebd9630f11bd08c28b7bd43c7bdfc62050654`.
- RocksDB `db_bench` SHA-256:
  `46eacd565df14c5e759811878b13271b3c0c4c01526a1c49404ceb180dd334cc`.
- Dataset: 1,000,000 unique 16-byte keys and deterministic incompressible
  100-byte values.
- Commit batch: 10,000 mutations; one warm-up and five alternating measured
  fresh processes per engine.
- Both engines use synchronous commits, an enabled WAL, no compression, and a
  256 MiB cache budget. RocksDB receives `--compression_ratio=1.0` and reports
  100 bytes after compression estimation.
- WSLDB uses a 64 MiB durable write buffer and explicitly drains pending work.
  RocksDB explicitly flushes and waits for compaction.

RocksDB was compiled from the clean tagged tree with `DEBUG_LEVEL=0`, RTTI,
native instructions, gflags, and without compression backends or jemalloc. No
RocksDB source was modified. Native instructions favor RocksDB relative to
WSLDB's default Rust target.

## Result

| Median metric | WSLDB | RocksDB | Outcome |
| --- | ---: | ---: | ---: |
| Active throughput | 620,753.047 ops/s | 559,712 ops/s | WSLDB +10.91% |
| Maximum durable-batch latency | 20.965 ms | 46.004 ms | WSLDB 54.43% lower |
| Active load time | 1.611 s | 1.787 s | WSLDB 9.85% lower |
| Maximum RSS | 205,258,752 B | 116,883,456 B | WSLDB 1.76x higher |
| Logical storage | 121,155,676 B | 121,951,652 B | WSLDB 0.65% lower |

WSLDB's median active-plus-drain interval was 1.771 seconds and its median
standalone drain was 157.055 milliseconds. Its median whole-process time was
2.63 seconds versus 7.05 seconds for RocksDB, but those intervals include
different verification and reporting work and are not treated as a
comparative win. Every WSLDB process reported zero checkpoint failures,
database write amplification of 2.091, and logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
The runner does not expose a comparable RocksDB write-amplification value.

Relative to the immediately preceding clean comparison, WAL key-prefix
encoding changed WSLDB's result from 1.23% larger than RocksDB to 0.65%
smaller. WSLDB retains the active-throughput and maximum durable-batch-latency
lead for this exact profile, but still uses 75.61% more peak RSS.

Other batch sizes, value sizes, reads, ranges, sustained workloads, cold-cache
behavior, Linux confirmation, power-loss testing, and the supported-platform
matrix remain required before any broad superiority or production-readiness
claim. Complete commands, manifests, raw outputs, resource reports, and sizes
are retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-wal-prefix-rocksdb-w2`.
