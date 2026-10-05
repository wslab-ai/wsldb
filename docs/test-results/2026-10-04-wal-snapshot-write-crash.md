# Streamed WAL snapshot write crash boundary

Date: 2026-10-04

## Risk

Explicit buffered flush can consolidate a transaction chain into one sorted
snapshot record. The record is emitted through a bounded staging buffer into a
temporary replacement inode. Existing crash coverage began after that inode
was synced and did not directly terminate the process after a staged write but
before the sync.

## Change and invariant

Every staged positioned write now exposes `write_buffer_snapshot_write`
immediately after the complete chunk reaches the temporary inode. A crash at
that point must leave the existing WAL path unchanged. Reopening must replay
the old transaction chain and return the exact pre-flush logical snapshot.

## Verification

The focused buffered snapshot replacement test now includes the new pre-sync
write boundary:

```text
cargo fmt --all -- --check
PASS

cargo test --features crash-injection --test write_buffer \
  interrupted_buffer_snapshot_replacement_preserves_the_logical_snapshot \
  -- --exact
PASS: 1 passed; 0 failed; finished in 0.37s
```

The subprocess terminated at the new hook; reopening replayed the still-active
WAL, compared all entries with the pre-flush model, and completed full database
verification.
