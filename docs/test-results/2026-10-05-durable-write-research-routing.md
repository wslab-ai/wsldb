# Durable-write research routing

Date: 2026-10-05. This document records a design-routing decision, not a
performance result or an adoption of another implementation.

## Current evidence

The current 30-minute APFS W6 diagnostic is the relevant unresolved case:
WSLDB is 9.90% below RocksDB 11.8.1 in active throughput, overwrite and
delete/recreate p99 are 25.34% and 22.61% higher, and final logical storage is
3.26% larger. Point-read p99, peak RSS, and final drain are already lower.
This is one sequential, warm-cache process pair and does not prove a general
ratio.

Exact short-run phase accounting attributes 90.011% of successful buffered
commit time to stable-storage synchronization and 5.825% to preflight. A later
routing diagnostic found no repeatable mean-sync advantage for either WSLDB or
RocksDB 10.10.1: the result changed sign between alternating pairs. Therefore
the evidence does not justify replacing the tree to optimize isolated strict-
durability commits.

## Primary-source review

| Design | Applicable result | Fit for the current W6 debt |
| :--- | :--- | :--- |
| [VeriBetrKV][veribetrkv] | A crash-safe CoW B-epsilon tree uses a logical journal so each sync need not write large dirty tree nodes. Its published insertion result remained eight times slower than RocksDB. | WSLDB already combines a logical WAL with a CoW tree and asynchronous checkpoint preparation. Changing the tree does not remove the mandatory WAL barrier. |
| [Jungle][jungle] | Batches sorted updates into CoW B+ trees, separates keys and values, and trades update cost against periodic-compaction space. | Most compelling when values are much larger than keys and updates can be batched. The failing W6 case uses 100-byte values and one durable mutation at a time. |
| [TRIAD][triad] | Exploits hot-key skew, defers and batches storage work, and avoids duplicate log/storage writes in an LSM engine. | Supports the existing logical-WAL/grouping direction. It does not establish that an LSM conversion would improve strict sequential commits. |
| [SILK][silk] | Schedules and preempts background flush/compaction I/O to reduce tail interference. | Directly relevant only if WSLDB mutation tails correlate with checkpoint or compaction activity. This is the first untested causal route. |
| [vLSM][vlsm] | Reduces LSM compaction-chain width and length to improve tail latency and I/O amplification. | Its mechanism is specific to SST/level compaction. WSLDB has neither levels nor an L0 compaction chain. |
| [BVLSM][bvlsm] | Separates 64 KiB values at WAL time and reports its largest gain with asynchronous WAL. | Not comparable to the frozen 100-byte, sync-on-every-mutation W6 contract. It may be reconsidered only for a preregistered large-value profile. |
| [CompassDB][compassdb] | Uses an in-memory two-tier perfect hash index for constant-time point lookup. | Conflicts with WSLDB's ordered range API and targets a point-lookup bottleneck that W6 evidence does not show. |
| [FastCommit][fastcommit] | Makes ext4 filesystem journaling more compact and selective. | This is a Linux filesystem feature, not a portable database algorithm. Linux evidence must record filesystem features and mount state rather than attribute the effect to WSLDB. |

No source above is performance evidence for WSLDB. Any implementation remains
subject to dependency and license review, the existing crash invariants, and a
separate preregistered experiment.

## Frozen routing decision

No storage-core rewrite is accepted from this review. The next performance
evidence must be collected on an isolated benchmark host, not the development
laptop, in this order:

1. Repeat alternating W5/W6 pairs against the pinned RocksDB 11.8.1 commit on
   the accepted WSLDB head. Preserve strict synchronous durability, identical
   operation streams, logical hashes, and the existing warm/cold-cache rules.
2. Record WSLDB WAL sync count and latency distribution, complete commit
   distribution, checkpoint count/failures/bytes, and the identity of every
   mutation window containing checkpoint publication. Do not infer a phase
   from whole-operation p99 alone.
3. Run the already preregistered concurrent group-commit comparison. Sequential
   and concurrent results remain separate claims.
4. If mutation-tail windows correlate with checkpoint work in a majority of
   repeated pairs, preregister one bounded checkpoint-I/O scheduling candidate
   inspired by the scheduling question in SILK. It must preserve the WAL size
   bound and explicit-flush semantics.
5. If at least 80% of isolated sequential commit time remains in WAL sync and
   checkpoint correlation is absent, reject CPU-only and tree-replacement work
   for W6. The remaining route is group amortization or documented filesystem/
   device behavior; weakening durability is forbidden.
6. Consider key/value separation or a write-optimized tree only after a
   separate large-value or write-amplification profile demonstrates that data
   movement, rather than the persistence barrier, is the measured bottleneck.

Any core candidate must retain ordered point/range semantics, snapshot
isolation, bounded recovery, strict synchronous durability, deterministic
encoding, corruption detection, backup and compaction safety, and the complete
crash/I/O-failure matrix. Acceptance requires a preregistered joint gate for
throughput, mutation p99, RSS, final storage, write amplification, and logical
equivalence. A headline result from a different workload cannot substitute for
that gate.

This routing decision adds no claim of RocksDB parity. The pinned alternating
matrix, supported-host cold-cache evidence, current Linux/ext4 release suite,
and physical power-loss matrix remain open.

The checkpoint-window instrumentation required by step 2 is now implemented
and verified independently of a timed workload
([evidence](2026-10-05-checkpoint-window-observability.md)). The sustained
runner also produces a fail-closed `checkpoint-windows.csv` by joining raw
per-window latency output with checkpoint lifecycle records; missing,
duplicated, non-contiguous, or malformed records invalidate the run. The
external benchmark run remains outstanding.

[veribetrkv]: https://www.usenix.org/conference/osdi20/presentation/hance
[jungle]: https://www.usenix.org/conference/hotstorage19/presentation/ahn
[triad]: https://www.usenix.org/conference/atc17/technical-sessions/presentation/balmau
[silk]: https://www.usenix.org/conference/atc19/presentation/balmau
[vlsm]: https://arxiv.org/abs/2407.15581
[bvlsm]: https://arxiv.org/abs/2506.04678
[compassdb]: https://arxiv.org/abs/2406.18099
[fastcommit]: https://www.usenix.org/conference/atc24/presentation/shirwadkar
