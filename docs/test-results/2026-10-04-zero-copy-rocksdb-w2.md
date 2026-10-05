# Zero-copy snapshot W2 comparison with RocksDB 11.8.1

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is evidence for one durable random-load profile, not a general claim that
WSLDB is faster or better than RocksDB.

## Reproducibility

- WSLDB commit: `2a66746a6877c14404a2999488f8435172382615`.
- WSLDB storage implementation: `f2a792c`.
- WSLDB benchmark SHA-256:
  `49fbecf164e71f3fd37bda11c6698d48c00e6ad4ad76ed03e44959398c03d7dc`.
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
| Active throughput | 640,883.223 ops/s | 538,271 ops/s | WSLDB +19.06% |
| Maximum durable-batch latency | 18.501 ms | 46.124 ms | WSLDB 59.89% lower |
| Active load time | 1.560 s | 1.858 s | WSLDB 16.02% lower |
| Maximum RSS | 203,669,504 B | 116,883,456 B | WSLDB 1.74x higher |
| Logical storage | 123,446,916 B | 121,951,653 B | WSLDB 1.23% higher |

WSLDB's median active-plus-drain interval was 1.736 seconds and its median
standalone drain was 156.548 milliseconds. Its median whole-process time was
2.57 seconds versus 7.14 seconds for RocksDB, but those intervals include
different verification and reporting work and are not treated as a
comparative win. Every WSLDB process reported zero checkpoint failures,
database write amplification of 2.150, and logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
The runner does not expose a comparable RocksDB write-amplification value.

Relative to the immediately preceding fresh comparison, streaming snapshot
handoff reduced WSLDB's median peak RSS from 314,867,712 to 203,669,504 bytes.
The remaining memory gap against RocksDB fell from 2.69x to 1.74x. Persisted
WSLDB main and WAL bytes stayed identical, so the gain did not change the file
format or defer work outside the measured process.

WSLDB therefore wins active throughput and durable-batch tail latency for this
exact profile, while still losing decisively on memory and slightly on storage.
Other batch sizes, values, reads, ranges, sustained workloads, cold-cache
behavior, Linux confirmation, power-loss testing, and the supported-platform
matrix remain required before any broad superiority or production-readiness
claim. Complete commands, manifests, raw outputs, resource reports, and sizes
are retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-zero-copy-rocksdb-w2`.
