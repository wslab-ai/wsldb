# Fallible group-overlay staging

Date: 2026-10-05

## Risk

Buffered group commit must determine whether every mutation inserts, replaces,
or deletes a key after applying all earlier batches in the same group. The
preflight overlay previously copied each distinct key into a
`BTreeMap<Vec<u8>, bool>`. Those key copies and collection nodes were allocated
incrementally before the WAL append, so a large valid group duplicated key
storage and retained an allocation path that could not be reserved fallibly.

## Change

The overlay is now a `HashMap<Arc<[u8]>, bool>` sized with the already computed
mutation upper bound. Reservation failure returns `Error::FileTooLarge` before
any WAL write. Insertions clone the immutable key handle already owned by each
`ChangeSet`; they no longer allocate and copy the key bytes. The map is used
only for presence lookup, so hash iteration order cannot affect transaction
ordering, entry counts, WAL encoding, or persisted bytes.

## Verification

Cargo work used two build jobs and a private temporary target directory. No
benchmark, soak, cache reset, shutdown, reboot, or power operation was run.

```text
cargo test --all-features --test group_commit
  12 passed; 0 failed

cargo test --all-targets --all-features
  273 passed; 0 failed

cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
git diff --check
scripts/release-audit.sh
  passed
```

This is allocation and correctness hardening, not a performance claim. The
external Linux, filesystem power-loss, cold-cache, and pinned RocksDB release
gates remain open.
