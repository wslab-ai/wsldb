# Automatic concurrent group-commit preregistration

Date: 2026-10-04. This contract is committed before implementation or
measurement.

## Observation

WSLDB's explicit `Database::commit_group` already writes independently
checksummed transaction records with one durability synchronization. It
requires one caller to collect every batch, however. `Database::write` also
holds the database handle's writer mutex for the complete transaction, so
unrelated concurrent blind writes cannot currently form groups automatically.

The single-threaded W6 profile cannot remove its one mandatory stable-storage
barrier without weakening durability. Concurrent writers can instead amortize
that barrier while preserving per-request synchronous completion.

## Candidate

Add `Database::commit_batch(WriteBatch) -> Result<TransactionId>` for blind
writes when the durable write buffer is enabled.

- Every caller enqueues exactly one ordered batch and waits for its own result.
- The first caller becomes a foreground leader. It immediately drains the
  current queue; there is no timer, sleep, background thread, or deliberate
  grouping delay.
- Requests arriving while a group is writing or synchronizing form the next
  group. The same leader drains successive non-empty groups before retiring.
- Queue order defines consecutive transaction order. Each successful caller
  receives the transaction ID assigned to its batch, not merely the final
  group ID.
- The existing writer mutex serializes automatic groups with ordinary write
  transactions, explicit groups, refresh, flush, and active compaction.
- The existing storage `commit_group` path remains the sole implementation of
  WAL encoding, size validation, durability, recovery, entry-count updates,
  and publication.
- A group failure is reported to every member with the same error category.
  In particular, an unconfirmed WAL sync remains `CommitOutcomeUnknown` for
  every request in that group.

The explicit `commit_group` API and persisted bytes remain unchanged.

## Frozen acceptance gate

Correctness requires focused coverage of:

- concurrent unique-key batches returning distinct consecutive transaction
  IDs and the exact final logical contents;
- empty batches advancing transaction IDs;
- serialization with an ordinary writer already holding the handle;
- rejection when the write buffer is disabled;
- injected pre-sync and sync failures reaching every grouped caller without a
  hang, followed by verified recovery;
- public API compilation and existing explicit-group recovery tests.

The dependency-free group benchmark will add an automatic concurrent mode.
On one short 16-thread, 4,096-transaction APFS diagnostic, the candidate must:

- preserve the same transaction count, entry count, and logical SHA-256 as
  sequential individual commits;
- use fewer than 50% as many WAL synchronizations;
- deliver at least 1.5 times the sequential transaction throughput;
- introduce no unmeasured drain or background work.

This diagnostic establishes only the mechanism. It does not replace a frozen
concurrent comparison against RocksDB on macOS and Linux, and it cannot change
the single-threaded W6 assessment.

## Result

Accepted. The implementation commit was
`fbc0b69f8e8021e66c69b103eac293a48f1bf4c8`. The clean optimized benchmark
binary SHA-256 was
`4cfd9efc2ae465229a08277c1f1f5d2ea555e58d3e8271cfda8bc189f7c5d267`.
One APFS run used the preregistered 4,096 transactions and 16 concurrent
callers:

| Mode | Transactions/s | WAL syncs | WAL bytes |
| :--- | ---: | ---: | ---: |
| Sequential individual commits | 185.457 | 4,096 | 802,816 |
| Explicit groups of 16 | 2,686.973 | 256 | 802,816 |
| Automatic 16-thread groups | 1,973.986 | 328 | 802,816 |

Automatic grouping delivered 10.644 times the sequential throughput and used
8.008% as many WAL synchronizations, passing the 1.5-times and below-50%
primary gates. All three modes finished at transaction 4,096 with the same
logical SHA-256
`cbd8ba5aeeb849af1dc5ba4c266622880cf776788942a74e7044c16a73d4b65d`
and the same WAL length. Every caller had returned before the measured interval
ended; the implementation has no background worker or delayed drain.

Focused tests passed for queue ordering and leadership handoff, concurrent
transaction IDs and verified reopen, empty batches, disabled buffering,
ordinary-writer serialization, partial-write rollback, shared-sync ambiguity,
explicit-group crash recovery, and the external public API. The all-target
all-feature check and Clippy with warnings denied also passed.

The automatic path is intentionally slower than a caller-prepared explicit
group because it does not delay the first request to collect a full group. The
result establishes a useful scalable API without trading away isolated-call
latency. It is not a RocksDB comparison and does not change the current W6
performance debt.
