# Exclusive-owner write path preregistration

Date: 2026-10-04. This document was committed before implementation or
candidate measurement.

## Observation

RocksDB exclusively owns an open database. WSLDB instead permits independent
processes to commit concurrently and therefore acquires the cross-process
commit lock, opens the current main-file path, and validates the physical head
for every buffered commit. A retained ten-second W5 profile attributed 344 of
6,824 main-thread samples to opening or closing that path. A previous candidate
that reused the main handle improved absolute W5 and W6 throughput by 4.62% and
4.96%, but was correctly rejected because its normalized cross-engine gains
missed that experiment's stricter gate.

The current 30-minute W6 result spends 90.325% of buffered commit time in WAL
encoding, append, and mandatory synchronization. The remaining ownership and
validation work is nevertheless large enough to account for much of the
observed 9.90% throughput deficit.

## Candidate

Add an explicit `DatabaseOptions` mode that holds the existing commit lock for
the lifetime of the storage generation. Commits through that owner still
serialize in-process, validate the logical base transaction, encode the same
WAL records, and execute the same stable-storage synchronization. Separately
opened handles and processes cannot publish until the exclusive owner closes.

Because external replacement is excluded, this mode may reuse its current
main-file handle and skip per-commit cross-process lock acquisition and path
reopening. The default remains multiprocess-capable. Active compaction,
refresh, flush, and checkpoint replacement must transfer the held lock without
an unlocked publication window.

This is a separate ownership mode, not a weakening of durability. The sustained
benchmark must expose it as an explicit diagnostic switch; existing W5/W6
evidence and default-mode claims remain unchanged.

## Acceptance gate

The implementation is accepted only if:

- focused tests prove exclusion of a second handle, lock transfer across active
  compaction, and unchanged reopen/recovery bytes;
- existing multiprocess, group-commit, checkpoint, compaction-race, crash, and
  fault tests pass in the default mode;
- a short same-fixture W6 diagnostic improves active throughput by at least 5%
  without increasing overwrite or delete/recreate p99 by more than 5%;
- the operation-limited semantic run preserves entry count and logical hash;
- formatting, all-target/all-feature tests and checks, Clippy with warnings
  denied, and `git diff --check` pass.

The short diagnostic only decides whether to retain the implementation. It
cannot replace the repeated 30-minute RocksDB comparison.

## Result

Rejected. Both focused compaction/lock-transfer tests passed, including a
buffered compact-reopen cycle and direct copy-on-write compaction. The
candidate kept the cross-process lock held across active-file replacement and
reopened with the expected data.

On independent copies of the same 1,000,000-entry post-prefix-WAL fixture, the
five-second default path completed 3,792 operations at 758.104 operations/s.
Its overwrite and delete/recreate p99 values were 10.665 and 17.400
milliseconds. The exclusive-owner candidate completed 3,695 operations at
737.518 operations/s: throughput regressed by 2.72%, overwrite p99 increased
by 2.71% to 10.954 milliseconds, and delete/recreate p99 increased by 9.63% to
19.076 milliseconds.

The candidate missed both the throughput gate and the delete-tail guardrail,
so its implementation and benchmark switch were removed. The result weakens
the hypothesis that per-commit ownership validation explains W6's deficit and
leaves WAL synchronization behavior as the higher-priority target.
