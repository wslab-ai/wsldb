# Durable group commit

`Database::commit_group` is an explicit throughput API for applications that
can prepare independent mutation batches before requesting durability.
`Database::commit_batch` automatically groups overlapping blind-write calls.
Both are available only when `DatabaseOptions::write_buffer_bytes` is nonzero.
The ordinary read-dependent `WriteTransaction` API and its one-sync-per-commit
contract are unchanged.

## Ordering and visibility

- Input order is transaction order. Every `WriteBatch`, including an empty
  one, advances the transaction ID exactly once.
- A later batch observes the existence effects of earlier batches when the
  resulting entry count is computed. Repeated keys use the later batch's value.
- Every batch remains a distinct logical transaction inside one checksummed,
  compact WAL group record. One physical group is limited to 65,536 logical
  transactions and 1 GiB before any bytes are written.
- The group record is issued in one positioned write and confirmed by one WAL
  sync. The database handle publishes only the final head after that sync.
- Readers created before the call remain on their old immutable snapshot.
  Readers created after a successful call observe the complete group.

This follows the standard WAL force rule: log records must reach stable
storage before their commit is acknowledged. Grouping changes the number of
forces, not the ordering or checksum rules. The design is consistent with the
grouping described by [RocksDB's WAL documentation][rocks-wal] and the WAL
foundation in [ARIES][aries]; WSLDB's implementation is independent.

[rocks-wal]: https://github.com/facebook/rocksdb/wiki/WAL-Performance
[aries]: https://www.cs.cmu.edu/~15849g/readings/mohan92.pdf

## Automatic grouping

The first `commit_batch` caller becomes a foreground leader and immediately
commits the batches already queued. Calls arriving while that group writes or
syncs form the next group. After its group completes, the leader hands
leadership to one member of the next queue and returns its own result. There is
no timer, deliberate delay, background worker, or work left after the final
caller returns.

Queue order is transaction order. Every caller receives the transaction ID of
its own batch and observes completion only after the shared WAL sync succeeds.
If more than 65,536 calls overlap, the coordinator drains consecutive bounded
groups without reordering or rejecting callers.
The database handle's existing writer mutex orders automatic groups with
ordinary transactions, explicit groups, refresh, flush, and compaction.

## Failure contract

| Boundary | Result |
| --- | --- |
| Invalid input, encoding limit, exhausted transaction ID, or stale head | Definite error; no group bytes are written |
| Partial positioned write | WAL is truncated to its previous boundary and that truncation is synced before a definite I/O error is returned |
| Truncation or rollback sync cannot be confirmed | `Error::CommitOutcomeUnknown` |
| WAL sync cannot be confirmed | `Error::CommitOutcomeUnknown` |
| Successful WAL sync followed by an automatic checkpoint failure | The group remains durable in WAL; `checkpoint_failures` increments |
| Successful return | Every batch through the returned transaction ID is durable |

A process or machine stop before the method returns can leave the old head or
the complete group. Recovery validates the complete physical group before
yielding any logical transaction and never accepts a torn group. After
`CommitOutcomeUnknown`, refresh or reopen and compare the durable transaction
ID with the base ID and submitted batch count before retrying.

An ambiguous automatic group fails its coordinator closed until a successful
`Database::refresh` installs the recovered durable head. Requests already
queued behind that group and new `commit_batch` calls return
`Error::LockPoisoned` without attempting another WAL append. A successful
refresh resets the coordinator; reopening creates a fresh coordinator.

Explicit groups have no hidden delay, background thread, or cancellation
window; their caller chooses the group size. Automatic groups likewise add no
delay, but their size is determined by naturally overlapping callers.

The automatic coordinator also owns leadership through a drop guard. If an
unexpected internal panic unwinds a foreground leader, the guard permanently
fails that coordinator closed: current and future waiters receive
`Error::LockPoisoned` instead of hanging or continuing from potentially
ambiguous in-memory WAL state. The panic still propagates to the affected
thread. Reopen the database before submitting more work.

## Observability

One explicit or automatic group contributes one operation to commit latency
and one operation to sync latency. `logical_mutation_bytes` covers every
mutation and `write_buffer_bytes` covers the compact physical record. The
durable transaction ID advances once per batch, so the ID delta records the
number of logical transactions even though latency counters record the shared
group operation.
