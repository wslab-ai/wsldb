# Delta catch-up compaction preregistration

Date: 2026-10-04. This contract is committed before implementation or
measurement.

## Problem and research route

WSLDB compaction builds a verified dense copy without blocking commits, but it
currently rejects publication if even one transaction commits after the
captured snapshot. Under a continuous buffered writer this makes compaction
unlikely to publish, so bounded storage still requires an operational pause.

The candidate combines three established ideas without copying another
engine's code: immutable snapshot construction, a delta chain for concurrent
changes, and a short consolidation phase before atomic publication. These are
the relevant mechanisms in LSM background compaction and delta-based B-tree
consolidation; WSLDB retains its copy-on-write B+ tree and WAL.

Primary references:

- Niv Dayan and Stratos Idreos, “The Log-Structured Merge-Bush & the Wacky
  Continuum”, SIGMOD 2019: <https://doi.org/10.1145/3299869.3319903>
- Justin DeBrabant et al., “A Prolegomenon on OLTP Database Systems for
  Non-Volatile Memory”, ADMS 2014, discussion of shadow paging and logging:
  <https://www.cs.cmu.edu/~pavlo/papers/adms14-nvm.pdf>
- Justin Levandoski, David Lomet, and Sudipta Sengupta, “The Bw-Tree: A B-tree
  for New Hardware Platforms”, ICDE 2013:
  <https://doi.org/10.1109/ICDE.2013.6544834>
- RocksDB compaction overview:
  <https://github.com/facebook/rocksdb/wiki/Compaction>

The references motivate immutable bases plus bounded deltas and consolidation;
they do not define WSLDB's recovery protocol.

## Frozen protocol

This candidate changes only active compaction of a database with durable write
buffering. Direct copy-on-write compaction keeps its existing conflict rule.

1. Under the commit and sidecar locks, capture logical head `T`, physical main
   checkpoint identity, WAL identity, and the exact durable WAL boundary.
2. Hold the existing main-file lease so a checkpoint cannot rotate the base.
   Build and fully verify a dense replacement representing `T` while WAL
   commits may continue.
3. Reacquire the short locks. Reject publication if the physical checkpoint or
   WAL identity changed, the WAL shrank below the captured boundary, or its
   valid transaction chain no longer contains `T`.
4. The dense snapshot has already passed full uncached verification. Derive one
   ordered net change set from versions newer than `T`, then apply only that
   delta through the same checksum-validating copy-on-write path used by a
   normal commit. Give it transaction ID `current + 1`, sync its data, and
   publish its metadata while commits remain blocked. Do not repeat a full-tree
   scan inside the short lock; full verification of the reopened result remains
   part of the operation before success is returned.
5. Atomically rename the replacement over the main path and sync its directory.
   Only after publication truncate and sync the old WAL. Reopen the new
   generation before releasing the operation.

The transaction increment is mandatory even when the delta is empty: it makes
old handles and page caches detect the generation rotation. A crash before
rename leaves the old main plus WAL authoritative. A crash after rename but
before WAL clearing leaves a main checkpoint newer than every WAL record;
recovery must discard that fully covered prefix. No two-file atomic rename is
assumed.

## Frozen acceptance gates

- A deterministic paused-compaction subprocess accepts buffered commits after
  snapshot capture, publishes successfully, and preserves every committed key,
  final entry count, logical SHA-256, and transaction ordering.
- The equivalent direct-COW race continues to return `WriteConflict`.
- Old readers remain valid; stale writers fail; the compacting handle or a
  fresh handle remains writable after publication.
- Focused failpoints before catch-up, after caught-up replacement sync, after
  main rename, and before/after WAL truncation recover either the complete old
  generation plus WAL or the complete new generation, never a partial state.
- I/O failures before rename are definite; failures after possible rename use
  `PublicationOutcomeUnknown`.
- A 100,000-key bounded diagnostic with concurrent buffered churn reduces
  final logical storage by at least 5%; the catch-up lock interval is reported,
  not hidden inside compaction wall time.
- A no-compaction W6 control must remain byte-identical and must not regress
  throughput or mutation p99 by more than 1%, because its foreground path
  should be unchanged.
- Formatting, complete all-feature tests, release crash/fault tests, all-target
  check, Clippy with warnings denied, and `git diff --check` pass.

This work targets bounded storage and compaction availability. It cannot by
itself establish RocksDB throughput superiority; a later pinned alternating
W6 run must measure the combined foreground and background policy.

## Result

Accepted at source commit
`582c22e7caa87113b5389616a99bec8423bda418`. The implementation keeps direct
copy-on-write compaction's stale-publication rejection and adds buffered WAL
tail refresh, a latest-value delta derived after the captured transaction, and
copy-on-write catch-up of the already verified dense replacement. Full
verification of the published generation runs after releasing the publication
locks, while both durability syncs remain inside the protected phase.

The deterministic race test committed after the compactor's snapshot capture.
Buffered compaction published successfully, preserved the concurrent value and
entry count, advanced the concurrent transaction by exactly one generation,
kept the old reader valid, rejected the stale writer, and reopened writable.
The equivalent direct race continued to return `WriteConflict`. Crash tests
covered catch-up data sync, metadata write and sync, rename, buffered
publication, WAL truncation, and WAL clearing. Injected I/O failures at all
three pre-rename catch-up stages left the original main file plus WAL
authoritative.

The bounded 100,000-key APFS diagnostic used an Apple M1 Max on macOS 26.6.2.
The exact compaction harness binary SHA-256 was
`837a727f30f616959182eb2263fcf2e4cd5769c0f190e924fdb08b0ac2885cec`.
While the dense snapshot was built, 208 independently durable overwrites
committed. Catch-up advanced source transaction 101 through concurrent
transaction 309 and published generation 310. The one reported conflict was
the expected stale-handle result after generation rotation.

| Metric | Result |
| --- | ---: |
| Compaction wall time | 1.415 s |
| Publication-lock pause | 49.649 ms |
| Publication share of wall time | 3.51% |
| Concurrent commit p99 | 17.137 ms |
| Concurrent commit maximum | 27.161 ms |
| Storage before | 337,825,988 B |
| Storage after | 12,177,408 B |
| Storage reduction | 96.40% |

The foreground no-compaction control used baseline commit `5b5b953` and the
accepted source with three alternating two-second W6 process pairs. Binary
SHA-256 values were
`0842eda9f6394b4048f1c28682658122a8755565a1970787044208519260ae4a`
for baseline and
`63b95c992af598be379c152fa3ca7044eaaf3dc372244e6c1d09eab63e6d2de3`
for candidate.

| No-compaction W6 median | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Active throughput | 747.176 ops/s | 786.679 ops/s | +5.29% |
| Overwrite p99 | 11.644 ms | 11.527 ms | -1.01% |
| Delete/recreate p99 | 24.598 ms | 22.998 ms | -6.50% |
| Maximum RSS | 17,072,128 B | 16,941,056 B | -0.77% |

A separate deterministic 1,000-operation W6 control produced the same 100,000
entries, logical SHA-256
`c2e2b95258cacf52ee35a30a70df789788e4311893202dd0e17917618561072f`,
main-file SHA-256
`8b70188018c3209d1dafe7aa49476bc5aea4a999e346c56f2f22d6cadc84f544`,
WAL SHA-256
`c891c86a1f81b4ff9a4a415d3c9727cf9ed458e79d042cf76bffb74582290d8a`,
and 12,202,866 logical storage bytes for both variants.

The complete all-target all-feature suite passed, including 70 unit tests and
every integration, multiprocess, crash, injected-I/O, maintenance, and
benchmark target. The focused compaction race matrix also passed in release
mode. All-target check, Clippy with warnings denied, formatting, and
`git diff --check` passed. Raw binaries, stdout, stderr, and exact-control
artifacts are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-delta-catch-up-compaction`.

This accepts online delta catch-up and closes the continuous-buffered-writer
publication gap. It does not establish an automatic compaction policy or a
general RocksDB performance win; those remain separate measured work.
