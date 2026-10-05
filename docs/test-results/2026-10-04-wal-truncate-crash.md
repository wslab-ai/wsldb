# WAL truncation crash boundary

Date: 2026-10-04

## Risk

Clearing a WAL changes its durable length with `set_len(0)` and then calls
`sync_all()`. The existing crash matrix covered the state before truncation and
the state after the sync, but not process termination between those operations.

## Change

The shared WAL truncation primitive now exposes a `wal_truncate` crash point
immediately after `set_len` and before `sync_all`. Both checkpoint publication
and buffered active compaction exercise the boundary. In either workflow the
new main-file metadata is already durable, so recovery must accept either the
old WAL length or the cleared WAL and return the same complete logical state.

## Verification

The focused checkpoint crash test reopens and fully verifies the committed
snapshot after termination at `wal_truncate`. The buffered-compaction crash
test independently checks the same boundary after atomic main-file replacement.

```text
cargo fmt --all -- --check
PASS

cargo test --features crash-injection --test buffer_checkpoint_crash \
  interrupted_buffer_checkpoints_recover_the_durable_commit -- --exact
PASS: 1 passed; 0 failed; finished in 0.80s

cargo test --features crash-injection --test write_buffer \
  interrupted_buffered_compaction_preserves_the_logical_snapshot -- --exact
PASS: 1 passed; 0 failed; finished in 0.59s
```

Both subprocess matrices reached the new failpoint, aborted, reopened the
database, compared every logical entry, and completed full verification.
