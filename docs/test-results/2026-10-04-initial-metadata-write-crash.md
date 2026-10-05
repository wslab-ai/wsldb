# Initial metadata write crash boundary

Date: 2026-10-04

## Risk

Initial creation writes the empty database metadata to a new temporary inode
before syncing and publishing it. The existing crash matrix started after the
file sync and therefore did not directly exercise termination after the
metadata write but before that sync.

## Change and invariant

The creation path now exposes `initial_metadata_write` immediately after the
positioned metadata write. A process terminated there must leave the final
database path absent. A subsequent open must create and fully verify a valid
empty database; the unsynced temporary inode must never become the live file.

## Verification

The focused initialization crash matrix includes the new boundary:

```text
cargo fmt --all -- --check
PASS

cargo test --features crash-injection --test initialization \
  interrupted_initialization_never_publishes_partial_metadata -- --exact
PASS: 1 passed; 0 failed; finished in 0.40s
```

The subprocess reached every configured boundary, aborted, and each subsequent
open returned a fully verified empty database.
