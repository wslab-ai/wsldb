# Current 30-minute W6 RocksDB diagnostic

Date: 2026-10-04.

## Subject

- WSLDB commit: `86a5421ec89d3485aefa540fb97d82f2a0aa1ab5`.
- RocksDB: clean official `v11.8.1` commit
  `abeebd9630f11bd08c28b7bd43c7bdfc62050654`.
- RocksDB adapter SHA-256:
  `492fc9798912fcde5ea4ad26df9c30fffba67c89a3d6e3ec854f7e6c1079d331`.
- Host: Apple M1 Max, macOS 26.6.2 (25G83), APFS on the internal SSD.
- Both runs used 1,000,000 keys, incompressible 100-byte values, a 256 MiB
  cache limit, the frozen Zipf-1.2 W6 operation stream, WAL, and synchronous
  durability for every mutation.

The WSLDB result is the current single-process 30-minute run documented
[separately](2026-10-04-current-w6-30m.md). The RocksDB adapter first loaded
the byte-identical initial dataset and reproduced its logical SHA-256. Its
timed process then performed the same warm preload, 1,800-second deadline,
per-second latency windows, periodic full logical hashes, final flush and
compaction, checksum verification, and independent reopen.

## Observed result

| Metric | WSLDB | RocksDB 11.8.1 | WSLDB relative result |
| --- | ---: | ---: | ---: |
| Active operations | 1,241,293 | 1,380,095 | 10.06% fewer |
| Active throughput | 700.324 ops/s | 777.302 ops/s | 9.90% lower |
| Wall-window throughput | 689.606 ops/s | 766.718 ops/s | 10.06% lower |
| Point-read p99 | 42.458 us | 107.042 us | 60.34% lower |
| Overwrite p99 | 13.020 ms | 10.388 ms | 25.34% higher |
| Delete/recreate p99 | 22.024 ms | 17.962 ms | 22.61% higher |
| Verification median | 946.578 ms | 822.279 ms | 15.12% higher |
| Final drain | 54.963 ms | 507.640 ms | 89.17% lower |
| Maximum RSS | 224,641,024 B | 319,553,536 B | 29.70% lower |
| Final logical storage | 126,006,865 B | 122,028,376 B | 3.26% higher |

Both engines preserved exactly 1,000,000 entries and passed their final full
verification and independent reopen. RocksDB completed 29 periodic
verifications. Its operation stream contained no window longer than two
seconds; per-second throughput ranged from 295.112 to 1,084.377 operations/s.
WSLDB's final storage is its main file plus consolidated WAL. RocksDB's final
storage includes all files after `WaitForCompact` flushed every column family
and waited for already scheduled background compaction and purge work. The
adapter does not request a manual full-range compaction.

This single sequential APFS diagnostic shows a real current WSLDB deficit on
the skewed durable-mutation workload. It does not establish a platform-wide
ratio or prove that one engine is generally superior. The frozen acceptance
comparison still requires repeated alternating runs, cold-cache coverage, and
supported-platform evidence. The result does narrow the next optimization
target: WSLDB's point reads, memory, and final drain already win here, while
durable overwrite/delete tails and full ordered verification remain slower.

Raw setup, run, resource, file-size, and independent-reopen evidence is
retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-rocksdb-w6-30m`.
The main RocksDB log SHA-256 is
`b8379603c333831654217c632c5b601b9b420bf7d968b425c8822d9387b927f5`.
