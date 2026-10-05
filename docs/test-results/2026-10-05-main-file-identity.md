# Main-file identity verification — 2026-10-05

Source commit under test:
`54d0b1f637117d8d95e3792551cb95da74d9c775`
(`fix: reject replaced main files on commit`).

## Risk and contract

A direct-mode handle pins an open descriptor for snapshot reads but reopens the
canonical main path for each serialized commit. Comparing only the decoded
`Head` was insufficient: an operator could atomically install a byte-identical
copy at the same path. The stale handle would then write new pages to the new
inode, publish the new head in memory, and continue reading through the old
descriptor. The two descriptors had equal metadata before the commit, so the
normal stale-head check did not detect the generation change.

On Unix, direct commit now compares the device and inode of the current path
descriptor with the descriptor pinned by the storage generation while holding
the stable interprocess writer lock. Any mismatch returns `WriteConflict`
before a page is allocated or written. `Database::refresh()` reopens the current
generation even when its logical transaction ID is unchanged; a later commit
then proceeds normally.

The regression test creates committed data, copies the database byte for byte,
atomically renames the copy over the main path, and proves that the stale commit
is rejected. It then refreshes the same handle, commits successfully, and
verifies that the rejected mutation is absent. Windows remains best-effort and
outside this Unix device/inode contract; stable Rust 1.89 does not expose the
corresponding by-handle file IDs without an unstable API.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak bodies were
not enabled. No reboot, shutdown, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --all-features --test path_identity
  8 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test database \
  stale_database_handle_gets_a_write_conflict
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test operations \
  active_compaction_keeps_the_database_handle_writable
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  225 tests passed; 0 failed

RUSTDOCFLAGS="-D warnings" CARGO_BUILD_JOBS=2 \
  cargo doc --no-deps --all-features
  passed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

The main-file and WAL encodings and the public Rust API are unchanged. This
closes the local equal-head replacement defect; it does not close the external
Linux, physical power-loss, or pinned RocksDB release gates.
