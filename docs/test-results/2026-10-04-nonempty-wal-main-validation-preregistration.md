# Non-empty WAL main-validation preregistration

Date: 2026-10-04. This acceptance contract is committed before candidate
implementation or measurement.

## Observation and invariant

The current 30-minute W6 run attributes 90.325% of buffered commit time to WAL
sync. The remaining work averages 545 microseconds per commit, while WSLDB is
9.90% behind RocksDB 11.8.1 on active throughput in that single diagnostic.
Every buffered commit currently opens the main database path and decodes its
metadata head before appending to the already-open WAL.

Under WSLDB's cross-process advisory commit lock, a non-empty current WAL is an
exclusion proof for the main file:

- direct commits and active compaction reject a non-empty WAL before changing
  main data or metadata;
- an external buffered commit changes the WAL length and is rejected by the
  existing exact append-boundary check;
- WAL snapshot publication changes its inode and is rejected by the existing
  current-file identity check;
- an in-process background checkpoint writes through the storage generation's
  existing main handle, serializes publication with commits, promotes the
  physical checkpoint, and returns a replacement storage generation when it
  clears or replaces the WAL.

The main path can change independently while the WAL is empty, so the first
buffered commit in every empty-WAL generation must retain the current
`open(main) + load_head` validation.

## Candidate

Ordinary and explicit group commits will inspect the serialized buffer state.
When its durable WAL length is non-zero, they will use the storage generation's
existing main handle and skip decoding main metadata. They will retain WAL
path identity, exact length, append-boundary, synchronous durability, base
transaction, entry-count, and point-membership checks. An empty WAL will keep
the existing fresh main-path open and metadata validation.

No public API or persisted byte changes are permitted.

## Frozen acceptance gate

The causal APFS A/B uses separately built baseline and candidate binaries,
100,000 initial keys, incompressible 100-byte values, a 256 MiB page cache, a
64 MiB durable buffer, and the frozen W5 uniform and W6 Zipf-1.2 mixes. Each
variant gets one unreported warm-up and seven measured 10-second fresh-file
processes per distribution, with variant order alternating by run.

The candidate is accepted only if:

- median active throughput improves by at least 3% on both W5 and W6;
- median overwrite and delete/recreate p99 regress by no more than 5% on
  either workload;
- median maximum RSS and final logical storage regress by no more than 1%;
- a separate operation-limited run preserves operation count, entry count,
  logical hash, main-file bytes, and WAL bytes;
- focused empty-WAL, non-empty-WAL, external direct-writer, external buffered
  writer, WAL replacement, background-checkpoint, compaction-race, refresh,
  crash, and I/O-failure tests pass;
- formatting, all-target debug and release tests, check, Clippy with warnings
  denied, and `git diff --check` pass.

If the short gate passes, the current 1M-entry long W6 comparison must be
repeated before the result can change the production-performance assessment.
The short causal run alone cannot establish RocksDB parity. If any short-gate
condition fails, the candidate and candidate-only tests will be removed while
the negative result remains documented.

## Result

Rejected and removed. Seven alternating measured processes per variant gave
the following medians:

| Workload | Metric | Baseline | Candidate | Candidate change |
| --- | --- | ---: | ---: | ---: |
| W5 | Active throughput | 784.604 ops/s | 815.826 ops/s | +3.98% |
| W5 | Overwrite p99 | 8.528 ms | 8.455 ms | -0.86% |
| W5 | Delete/recreate p99 | 15.542 ms | 14.564 ms | -6.29% |
| W5 | Maximum RSS | 20,332,544 B | 20,348,928 B | +0.08% |
| W5 | Logical storage | 12,439,989 B | 12,447,735 B | +0.06% |
| W6 | Active throughput | 803.020 ops/s | 824.675 ops/s | +2.70% |
| W6 | Overwrite p99 | 8.783 ms | 8.064 ms | -8.18% |
| W6 | Delete/recreate p99 | 15.538 ms | 13.503 ms | -13.09% |
| W6 | Maximum RSS | 20,037,632 B | 20,103,168 B | +0.33% |
| W6 | Logical storage | 12,313,878 B | 12,315,574 B | +0.01% |

The candidate won every paired measured process and passed every secondary
guardrail, but W6 missed the preregistered 3% throughput threshold. Its 2.70%
gain is not rounded up or accepted post hoc. A separate 5,000-operation W6
conformance run produced the same 100,000 entries, logical hash, main-file
SHA-256, and WAL SHA-256 for both variants. Focused empty-WAL, stale direct,
and group-commit tests passed before the candidate was removed.

The baseline binary SHA-256 was
`584140f31ebd949eab3a0ac28673cabcdd04f3bfa080eee41c6d69d7cac34ed1`; the
candidate binary SHA-256 was
`13e70d8afc3d9e496713346d14389c7a62eaf473fe2d98d55594bcde30b2d738`.
Raw logs and hashes are retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-nonempty-wal-main-validation-ab`.
The measured mechanism may be combined with a disjoint candidate only under a
new preregistered compound gate; it is not present in production code.
