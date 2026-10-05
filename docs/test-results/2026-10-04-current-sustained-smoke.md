# Current sustained-update smoke and conformance

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
These five-second results are diagnostics, not the long-duration production
gate.

## Reproducibility

- WSLDB commit: `b312797317e7e6832e1e060d9d316e5eb6277230`.
- WSLDB benchmark SHA-256:
  `584140f31ebd949eab3a0ac28673cabcdd04f3bfa080eee41c6d69d7cac34ed1`.
- RocksDB source: clean official tag `v11.8.1`, commit
  `abeebd9630f11bd08c28b7bd43c7bdfc62050654`.
- RocksDB adapter source SHA-256:
  `7006a3464eccbe6dc3be22f8b70d6a332612ed62143e7c2eda2330f0bbce9f0a`.
- RocksDB adapter binary SHA-256:
  `492fc9798912fcde5ea4ad26df9c30fffba67c89a3d6e3ec854f7e6c1079d331`.
- RocksDB static library SHA-256:
  `41177ab85f0a88015f60298796e6e74a57f83caf048788ed3526540bd5ef3181`.
- Both adapters used 100,000 entries, deterministic incompressible 100-byte
  values, a complete warm preload, a 256 MiB cache limit, and synchronous
  WAL-backed durability for every mutation.
- WSLDB used its 64 MiB durable write buffer. Each distribution used one
  warm-up and three alternating measured processes, with full verification
  every two active seconds and a separately measured final drain.

RocksDB was rebuilt with release optimization, native instructions, RTTI,
gflags, and compression backends disabled. On this macOS build both RocksDB's
`Sync` implementation and Rust's `File::sync_all` use `F_FULLFSYNC` for the
WAL, so the comparison does not give WSLDB a weaker synchronization primitive.

## W5 uniform

| Median metric | WSLDB | RocksDB | WSLDB relative result |
| --- | ---: | ---: | ---: |
| Active throughput | 721.705 ops/s | 801.750 ops/s | 10.0% lower |
| Point-read p50 | 8.000 us | 9.042 us | 11.5% lower |
| Point-read p99 | 67.084 us | 154.917 us | 56.7% lower |
| Overwrite p99 | 10.754 ms | 8.430 ms | 27.6% higher |
| Delete/recreate p99 | 18.402 ms | 15.201 ms | 21.1% higher |
| Final drain | 11.328 ms | 19.802 ms | 42.8% lower |
| Maximum RSS | 19,693,568 B | 23,937,024 B | 17.7% lower |
| Final logical storage | 12,335,871 B | 12,387,589 B | 0.42% lower |

## W6 Zipf 1.2

| Median metric | WSLDB | RocksDB | WSLDB relative result |
| --- | ---: | ---: | ---: |
| Active throughput | 734.102 ops/s | 794.266 ops/s | 7.6% lower |
| Point-read p50 | 2.208 us | 4.125 us | 46.5% lower |
| Point-read p99 | 36.042 us | 138.459 us | 74.0% lower |
| Overwrite p99 | 10.066 ms | 9.499 ms | 6.0% higher |
| Delete/recreate p99 | 21.215 ms | 28.543 ms | 25.7% lower |
| Final drain | 14.250 ms | 18.237 ms | 21.9% lower |
| Maximum RSS | 19,644,416 B | 23,838,720 B | 17.6% lower |
| Final logical storage | 12,285,159 B | 12,329,239 B | 0.36% lower |

WSLDB now wins warm read latency, peak memory, final drain, and logical storage
in both short profiles. It still loses total throughput and some durable
mutation tails. This is a materially different resource result from the older
smoke because the accepted arena, lazy-index, freeze-and-swap, and streaming
snapshot work removed most write-generation residency.

## Semantic conformance

A separate operation-limited run used 10,000 initial entries and exactly 1,000
operations. Both independent processes per engine and distribution reached
the limit with identical entry counts and hashes:

| Distribution | Operations | Entries | Logical SHA-256 |
| --- | ---: | ---: | --- |
| Uniform | 1,000 | 10,000 | `ff61aa66a4b6710e445aae1fe1090c7369a88093934eb02811f9251d29ca9fa7` |
| Zipf 1.2 | 1,000 | 10,000 | `d5ee4be0cd25cbc644e625141a4bc3e007f800241043015dcaa195490e121c2d` |

This proves adapter equivalence for the deterministic prefixes, not real
power-loss safety or long-duration performance. Raw commands, manifests,
outputs, resource reports, and size tables are retained at:

- `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-current-sustained-smoke`
- `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-current-sustained-conformance`
