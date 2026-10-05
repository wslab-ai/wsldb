# Freeze-and-swap checkpoint preparation

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation

The accepted W2 path reaches 524,188.733 operations/s in its latest fresh
paired RocksDB session, but its median maximum durable-batch latency is 297.086
milliseconds. RocksDB reaches 491,990 operations/s and 46.309 milliseconds on
the same workload. A one-MiB sequential page staging buffer did not reduce the
WSLDB pause, confirming that the remaining tail is not primarily positioned
write call overhead.

The current automatic checkpoint constructs, syncs, publishes, and retires a
complete buffered generation on the foreground commit that crosses the
configured threshold. The earlier asynchronous-publication experiment kept
the complete mutable state live while reconstructing a copied suffix; it
regressed throughput and nearly doubled process RSS when drained.

## Candidate

At the threshold, the foreground path will freeze the complete buffered
generation and immediately install a new mutable partition. The worker owns a
shared immutable reference to the frozen partition; keys and values are not
copied. New commits append to the same durable WAL and update only the new
partition while the worker constructs and syncs pages for the frozen head.

Only one process may prepare main-file pages for a database generation. A
separate advisory checkpoint lease must serialize background preparation with
checkpoint, compaction, and active-file replacement without holding the short
commit lock during construction. Failure to acquire the lease skips background
work and leaves the durable WAL authoritative.

Publication reacquires the commit lock, verifies the physical checkpoint and
WAL boundary, publishes the already-synced metadata, and only then replaces
the WAL with a deterministic snapshot of the post-freeze suffix. This ordering
keeps both crash alternatives recoverable: the old checkpoint with the full
WAL, or the new checkpoint with either the full WAL or its complete suffix.
An external commit or generation change makes a prepared result stale and it
must not be published.

Old read transactions retain their original store and frozen partition.
Explicit `flush()` drains preparation and publication. Normal commits apply
bounded backpressure if a worker cannot keep up, so memory use and the durable
WAL cannot grow without limit. Worker failure records a checkpoint failure and
falls back to the WAL without changing commit durability.

## Frozen correctness gate

The candidate is rejected unless all existing all-feature tests pass and new
tests prove all of the following:

- readers pinned before freeze, during preparation, and after publication see
  exactly their committed snapshots;
- commits made during preparation survive reopen and appear after the frozen
  checkpoint in transaction order;
- two processes cannot prepare overlapping main-file page ranges;
- an external commit or compaction prevents stale checkpoint publication;
- explicit flush drains a pending worker and returns one fully recoverable
  logical head;
- worker panic, page-write failure, data-sync failure, metadata failure, WAL
  replacement failure, and crashes at every publication boundary recover a
  complete old or new state;
- background work is joined or safely abandoned on handle replacement and
  drop, with no writer continuing against a replaced inode;
- no `unsafe` is introduced and every hand-written source file remains at or
  below 700 physical lines.

## Frozen performance gate

The causal A/B uses one million unique 16-byte keys, 100-byte values, 10,000
mutations per synchronous commit, a 64 MiB durable write buffer, and 65,536
decoded cache pages. It runs one warm-up and five measured fresh-file
processes per variant, alternating measured variants. Every candidate run must
drain pending background work before its final storage and recovery checks.

The candidate is accepted only if median maximum commit latency decreases by
at least 40%, median active W2 throughput does not regress by more than 2%,
median active-plus-drain time does not regress by more than 3%, median maximum
RSS grows by no more than 10%, and final logical storage does not exceed the
baseline. Logical hashes must match across all runs; persisted output must be
deterministic within each variant and reopen to the same verified snapshot.

Maximum sync latency, write amplification, checkpoint failures, worker drain
time, final main/WAL lengths, and cold reopen time are mandatory diagnostics.
The complete format, test, Clippy, check, and diff-check gate must pass.
Thresholds will not change after candidate results are observed.

## Result: accepted

The final candidate passed every frozen performance condition. One final-code
warm-up per variant preceded five measured fresh-file processes per variant;
measured variants alternated. The baseline was commit `f4cd259` with the same
drain instrumentation applied only to its benchmark harness.

| Median metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Active W2 throughput | 545,160.605 ops/s | 614,046.168 ops/s | +12.64% |
| Maximum commit latency | 239.934 ms | 18.558 ms | -92.27% |
| Active plus drain time | 2.154 s | 1.970 s | -8.54% |
| Maximum RSS | 298,680,320 B | 316,162,048 B | +5.85% |
| Drain latency | 318.320 ms | 343.289 ms | +7.84% |
| Total commit time | 1.493 s | 1.305 s | -12.60% |
| Maximum sync latency | 38.161 ms | 38.867 ms | +1.85% |
| Cold reopen to first read | 340.691 ms | 349.537 ms | +2.60% |

The primary requirement was at least a 40% maximum-commit reduction; the
candidate achieved 92.27%. Active throughput improved rather than consuming
its 2% allowance, active-plus-drain time improved rather than consuming its 3%
allowance, and RSS stayed inside the 10% guardrail. The longer standalone
drain is fully recovered by the shorter active interval. All measured runs
reported zero checkpoint failures and write amplification of 2.150.

All ten runs produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`,
main-file hash
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`,
and WAL hash
`21ba0081c157f7ca7a34adc93fd26e72c8ea3623abff1261d0be43cc9081eedd`.
Main-file and WAL lengths were respectively 65,486,848 and 57,960,068
bytes for every baseline and candidate run. Separate cold processes reopened
every drained file and completed the first lookup successfully.

The final protocol freezes without copying, constructs pages on a worker,
serializes its data sync with foreground and cross-process durability barriers,
publishes metadata before the WAL suffix, and releases the frozen generation
before suffix snapshot encoding. Focused tests cover reads and commits during
preparation, explicit drain, stale external publication, worker panic,
injected I/O failures, crash boundaries, reopen, and old-reader preservation.

An initial measurement sequence was discarded after the filesystem returned
`ENOSPC`; its logs are retained with an `invalid-enospc-` prefix. Before the
reported sequence, only experiment-created warm-up files were removed. Each
reported database was hashed, cold-opened, and then removed before the next
variant so free space remained constant. Raw output, hashes, and both binaries
are retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-freeze-swap-checkpoint-w2-ab`.
