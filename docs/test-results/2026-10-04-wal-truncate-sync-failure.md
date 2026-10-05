# WAL truncation sync failure semantics

Date: 2026-10-04

## Risk

A checkpoint or buffered active compaction publishes and syncs a new main-file
snapshot before clearing its obsolete WAL. An error while clearing the WAL was
previously returned as ordinary `Error::Io`, which could incorrectly suggest a
definite pre-publication failure even though durable state had already changed.
The sync after `set_len(0)` also had no dedicated I/O-failure boundary.

## Change and invariant

WAL truncation now exposes `wal_truncate_sync` after the length change and
before `sync_all`. Once checkpoint metadata or a compact generation has been
published, both a pre-truncate cleanup error and a truncate-sync error map to
`Error::PublicationOutcomeUnknown`. The caller must refresh or reopen before a
retry. Recovery must return the complete published logical snapshot whether
the old WAL length or the cleared length survives. The process-local physical
checkpoint is promoted only after WAL cleanup succeeds, so a handle that
receives this error fails closed with `WriteConflict` until refreshed.

## Verification

Focused checkpoint and buffered active-compaction fault tests inject the new
failure, require `PublicationOutcomeUnknown`, refresh or reopen, and fully
verify the published value:

```text
cargo test --features fault-injection --test maintenance_failures \
  checkpoint_wal_clear_failures_report_ambiguous_publication -- --exact
PASS: 1 passed; 0 failed; finished in 0.24s

cargo test --features fault-injection --test maintenance_failures \
  buffered_compaction_wal_sync_failure_reports_ambiguous_publication -- --exact
PASS: 1 passed; 0 failed; finished in 0.10s

cargo test --test write_buffer \
  automatic_buffer_checkpoints_preserve_old_readers -- --exact
PASS: 1 passed; 0 failed; finished in 0.14s

cargo test --test write_buffer \
  explicit_flush_consolidates_buffer_without_changing_snapshots -- --exact
PASS: 1 passed; 0 failed; finished in 0.08s
```

The checkpoint case covers both failure immediately before WAL clearing and
failure of the sync after its length change. The compaction case covers the
latter after an atomically renamed and directory-synced main generation. In
both cases the stale live handle rejects a write with `WriteConflict`; after an
explicit `refresh`, a new durable commit succeeds.
