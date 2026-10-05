# Single WAL length-check preregistration

Date: 2026-10-04. This acceptance contract is committed before candidate
implementation or measurement.

## Observation and candidate

The current 30-minute W6 result attributes approximately 90.3% of buffered
commit time to the mandatory WAL sync. WSLDB is 9.90% behind RocksDB 11.8.1 in
active throughput on that profile, while its overwrite and delete/recreate p99
are 25.34% and 22.61% higher. Point-read p99, maximum RSS, and final drain are
already materially lower.

Every buffered commit currently obtains the open WAL length twice. The
preflight `buffer_file_matches` check validates the WAL path identity and
length, then `wal::append` obtains the length again immediately before its
positioned write. Both calls occur under the process-local writer mutex and
the cross-process advisory commit lock. The second check is the authoritative
write-boundary guard and returns `WriteConflict` before writing if another
process has advanced the WAL.

The candidate will let ordinary and group buffered commits preflight only the
WAL path identity. Their existing `wal::append` or `wal::append_group` call
will retain the one authoritative length check. Refresh, flush, checkpoint,
backup, compaction, and recovery paths will retain both identity and length
validation because they do not necessarily proceed through an append.

The change must preserve synchronous durability, external-writer rejection,
transaction ordering, checksums, recovery, every persisted byte, and the
public API.

## Frozen acceptance gate

The causal APFS A/B uses separately built baseline and candidate binaries,
100,000 initial keys, incompressible 100-byte values, a 256 MiB page cache, a
64 MiB durable buffer, and the frozen W5 uniform and W6 Zipf-1.2 mixes. Each
variant gets one unreported warm-up and seven measured 10-second fresh-file
processes per distribution, with variant order alternating by run.

The candidate is accepted only if:

- median active throughput improves by at least 1.5% on both W5 and W6;
- median overwrite and delete/recreate p99 regress by no more than 5% on
  either workload;
- median maximum RSS and final logical storage regress by no more than 1%;
- a separate operation-limited conformance run preserves operation count,
  entry count, logical hash, main-file bytes, and WAL bytes;
- a focused test proves that an unexpected complete or incomplete external
  WAL append still prevents a buffered commit from publishing;
- formatting, all-target tests, check, Clippy with warnings denied, and
  `git diff --check` pass.

These short causal measurements cannot satisfy the long-duration production
gate or establish parity with RocksDB. If any acceptance condition fails, the
candidate implementation and candidate-only test will be removed while the
negative result remains documented.

## Result

Rejected and removed. Seven alternating measured processes per variant gave
the following medians:

| Workload | Metric | Baseline | Candidate | Candidate change |
| --- | --- | ---: | ---: | ---: |
| W5 | Active throughput | 772.214 ops/s | 787.010 ops/s | +1.92% |
| W5 | Overwrite p99 | 8.611 ms | 8.452 ms | -1.84% |
| W5 | Delete/recreate p99 | 15.641 ms | 14.872 ms | -4.92% |
| W5 | Maximum RSS | 20,316,160 B | 20,398,080 B | +0.40% |
| W5 | Logical storage | 12,437,206 B | 12,441,078 B | +0.03% |
| W6 | Active throughput | 809.877 ops/s | 799.749 ops/s | -1.25% |
| W6 | Overwrite p99 | 8.549 ms | 8.985 ms | +5.10% |
| W6 | Delete/recreate p99 | 16.096 ms | 15.628 ms | -2.91% |
| W6 | Maximum RSS | 20,054,016 B | 20,152,320 B | +0.49% |
| W6 | Logical storage | 12,314,120 B | 12,313,878 B | -0.002% |

W5 passed the throughput threshold, but W6 failed it and also exceeded the
overwrite-p99 guardrail. A separate 5,000-operation W6 conformance run produced
the same 100,000 entries, logical hash, main-file SHA-256, and WAL SHA-256 for
both variants. Focused stale-tail and group-commit tests also passed before the
candidate was removed. Those correctness results do not override the failed
performance gate.

The baseline binary SHA-256 was
`584140f31ebd949eab3a0ac28673cabcdd04f3bfa080eee41c6d69d7cac34ed1`; the
candidate binary SHA-256 was
`9ce8b8afa932242085c34e5f1aeffc9b1eac09909120817bda9ff6ecd5e1fda6`.
Raw logs and hashes are retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-single-wal-length-check-ab`.
The result shows that the duplicate WAL-length metadata lookup is not the
cause of the sustained W6 deficit.
