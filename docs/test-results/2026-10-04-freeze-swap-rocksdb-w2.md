# Freeze-and-swap W2 comparison with RocksDB 11.8.1

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is evidence for one durable random-load profile, not a general claim that
WSLDB is faster or better than RocksDB.

## Reproducibility

- WSLDB storage implementation: commit `9405a78f31147d9d240c76a734c846eaa2fb7856`.
- WSLDB comparison-harness commit: `03b5e5139ef9c4ddf7831645203838943bae683f`.
  The only intervening change adds RocksDB's explicit incompressible-value flag.
- WSLDB benchmark binary SHA-256:
  `f662a81ce635fdd0d32a406b25c51d5900de50a88f1b7f9609d1221801dc4759`.
- RocksDB source: official tag `v11.8.1`, commit
  `abeebd9630f11bd08c28b7bd43c7bdfc62050654`.
- RocksDB `db_bench` SHA-256:
  `46eacd565df14c5e759811878b13271b3c0c4c01526a1c49404ceb180dd334cc`.
- Dataset: 1,000,000 unique 16-byte keys and deterministic 100-byte values.
- Commit batch: 10,000 mutations; one warm-up and five alternating measured
  fresh processes per engine.
- Both engines use synchronous commits, an enabled WAL, no compression, and a
  256 MiB cache budget. RocksDB receives `--compression_ratio=1.0`; its own
  output confirms 100 bytes after compression estimation.
- WSLDB uses a 64 MiB durable write buffer and explicitly drains its pending
  worker after the active interval. RocksDB explicitly flushes and waits for
  compaction.

RocksDB was compiled from the clean tagged tree with `DEBUG_LEVEL=0`, RTTI,
native instructions, gflags, and without compression backends or jemalloc.
Because the host lacked space for a second copy of all object bytes in a static
archive, the same official release objects were linked directly into the final
executable. No RocksDB source was modified. Native instructions favor RocksDB
relative to WSLDB's default Rust target.

## Result

| Median metric | WSLDB | RocksDB | Outcome |
| --- | ---: | ---: | ---: |
| Active throughput | 625,008.284 ops/s | 508,787 ops/s | WSLDB +22.84% |
| Maximum durable-batch latency | 17.958 ms | 45.028 ms | WSLDB 60.12% lower |
| Active load time | 1.600 s | 1.965 s | WSLDB 18.58% lower |
| Maximum RSS | 314,867,712 B | 116,916,224 B | WSLDB 2.69x higher |
| Logical storage | 123,446,916 B | 121,951,653 B | WSLDB 1.23% higher |

WSLDB's median active-plus-drain interval was 1.917 seconds and its median
standalone drain was 317.473 milliseconds. Its median whole-process time was
2.79 seconds versus 7.24 seconds for RocksDB, but that interval includes
different reporting and verification work and is not treated as a comparative
win. Every WSLDB process reported zero checkpoint failures, database write
amplification of 2.150, and logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
The runner does not yet expose a comparable RocksDB write-amplification value.

The measured candidate therefore beats RocksDB on active throughput and
durable-batch tail latency for this exact profile, while decisively losing on
memory and slightly losing on storage. The RSS gap, missing comparative write
amplification, other batch sizes, reads, ranges, sustained workloads,
cold-cache behavior, and the supported-platform matrix still block a general
superiority claim.

Two earlier diagnostics are intentionally excluded from the result: one used
batch size 1,000 instead of the frozen 10,000, and one inherited RocksDB's
default 0.5 value-compressibility setting. Their raw logs remain alongside the
accepted run. Complete commands, manifests, outputs, resource reports, and
sizes for the accepted run are retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-final-rocksdb-comparison/w2-b10000-incompressible-final`.
