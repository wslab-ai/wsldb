# Packed write-buffer version arena

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

The current in-memory version record occupies 40 bytes on this 64-bit target:
`Option<VersionId>` is 16 bytes because every `usize`, including zero, is a
valid ID. Each live value also has its own boxed allocation. At the end of W2,
the durable buffer retains hundreds of thousands of records and values, which
contributes to the current 1.54-times RSS gap versus RocksDB.

The candidate will encode version IDs as nonzero one-based indexes, reducing a
version record to 32 bytes. Value bytes will be appended to one arena vector;
records store checked offset/length references and tombstones store no range.
Reallocation cannot invalidate references because only integer offsets are
retained. Every read validates its range before exposing bytes.

Version-chain order, snapshot visibility, tombstones, WAL bytes, checkpoint
ordering, point-index keys, and all persistent-format fields remain unchanged.

## Frozen acceptance gate

The causal A/B uses one-million unique 16-byte keys, 100-byte values, 10,000
mutations per synchronous commit, a 64 MiB durable write buffer, and 65,536
decoded cache pages. It runs one warm-up and five measured fresh-file processes
per variant, alternating measured variants.

This memory-focused candidate is accepted only if median maximum RSS decreases
by at least 5%, median active W2 throughput does not regress by more than 2%,
every main-file, WAL, and logical hash matches, layout and invalid-range tests
pass, all snapshot/history, checkpoint, crash, corruption, and recovery tests
pass, and the complete formatting, all-target all-feature test, Clippy, check,
and diff-check gate passes. Commit and sync latency are reported. Thresholds
will not be changed after results are observed.

## Result

Accepted. The preregistered one-warm-up, five-measured-run alternating A/B was
run from fresh files. Median maximum RSS decreased from 176,881,664 to
159,367,168 bytes, a 9.90% reduction. Median active W2 throughput increased
from 473,281.327 to 474,736.680 operations/s, a 0.31% improvement. Median
maximum commit latency decreased from 306,246,959 to 261,209,750 nanoseconds,
while median total commit time increased by 0.88%. Median recorded
`sync_total_ns` increased by 2.00% and median maximum sync latency increased by
4.00%; both remain secondary diagnostics rather than acceptance metrics for
this memory-focused candidate.

All ten measured runs produced the same logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`, main-file
hash `f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`, and WAL
hash `112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68`.
Main-file and WAL lengths remained 65,486,848 and 57,963,128 bytes. Raw output
is retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-packed-version-arena-w2-ab`.

Layout tests confirm that a version record is 32 bytes and an optional version
ID is one machine word on the measured target. Invalid value ranges fail
closed. The complete all-target, all-feature test suite, Clippy with warnings
denied, `cargo check`, format check, and diff check passed. Persistent bytes,
snapshot semantics, and the durability protocol are unchanged.
