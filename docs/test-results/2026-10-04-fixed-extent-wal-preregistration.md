# Fixed-extent WAL diagnostic preregistration

Date: 2026-10-04. This document was committed before the diagnostic harness or
its measurements.

## Observation

Exact WSLDB instrumentation on a clean two-second W6 diagnostic attributed
94.038% of successful buffered-commit time to `File::sync_all`, 1.877% to the
preceding WAL write system call, and 4.085% to all remaining commit work. The
193 commits averaged 10.001 milliseconds end to end, including 9.405
milliseconds in stable-storage synchronization.

RocksDB documents that extending a WAL can require both data and inode-size
metadata I/O. Its recycled-log option instead overwrites already allocated file
space so the file size does not change. RocksDB also notes that ordinary block
preallocation alone does not necessarily eliminate the metadata I/O.

## Diagnostic

Add an opt-in dependency-free benchmark with two byte-identical paths:

- `growing` starts with an empty, already synced file, then writes each record
  at the current end and calls `File::sync_all`;
- `fixed` writes and syncs the complete extent before measurement, seeks back
  to zero, then overwrites the same offsets and calls `File::sync_all` after
  every record without changing EOF.

Both paths use ordinary sequential `File::write_all`, identical 256-byte
records, the same number of synchronization calls, independent files, and
alternating order. File creation and fixed-extent initialization are outside
the measured interval. Every resulting record is read back and compared.

## Decision gate

Fixed extents justify a production recycled-WAL design only if:

- all byte verification passes;
- median per-record latency improves by at least 10% in every measured pair;
- p99 does not regress in any pair;
- formatting, benchmark compilation, and `git diff --check` pass.

This microbenchmark isolates EOF growth on one filesystem. It cannot establish
database throughput or replace W6, crash-recovery, and multiprocess gates.

## Result

The diagnostic ran once from clean commit
`b7401e9cabb2896da971bf383fcc01107b95b994` on macOS 26.6.2 arm64. The
optimized benchmark executable SHA-256 was
`538f65585cc4c43b99f54817b7e47f9589c9e91c4f8b10ada55a5a2aa9817428`.
It used the preregistered defaults of three alternating pairs and 64 records
per mode. All six files passed exact readback and length verification.

| Pair | First mode | Growing p50 | Fixed p50 | p50 improvement | Growing p99 | Fixed p99 | p99 result |
| ---: | :--- | ---: | ---: | ---: | ---: | ---: | :--- |
| 0 | growing | 4.443 ms | 5.104 ms | -14.87% | 8.214 ms | 9.690 ms | 17.96% worse |
| 1 | fixed | 5.056 ms | 4.991 ms | 1.27% | 10.142 ms | 9.182 ms | 9.47% better |
| 2 | growing | 5.412 ms | 5.158 ms | 4.69% | 8.991 ms | 13.233 ms | 47.18% worse |

The fixed extent failed both performance gates: no pair reached the required
10% median improvement, and two pairs regressed at p99. The result also varied
with order, which is consistent with the already observed short-run
synchronization noise. EOF growth is therefore not a demonstrated dominant
cause of WSLDB's durable-write debt on this system.

Do not add recycled-WAL tail metadata, recovery states, or file-format
complexity from this result. Retain the opt-in harness as a bounded filesystem
diagnostic; it may be rerun on release filesystems but is not a release claim.
