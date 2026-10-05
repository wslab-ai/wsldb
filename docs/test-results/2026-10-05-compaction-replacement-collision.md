# Compaction replacement collision verification — 2026-10-05

Source commit under test:
`2c40cce49104152d1aa15ab1ffbfb8212634e29b`
(`fix: preserve colliding compaction replacements`).

## Risk and contract

Active compaction uses an internal `*.wsldb-replacement-<pid>-<sequence>` path
before atomically renaming the verified generation over the main database. The
sequence restarts in each process. Previously the allocator returned its first
candidate without checking the directory. If a complete orphan from an earlier
process occupied that name, no-clobber publication correctly failed, but the
caller then unconditionally removed the path. It could therefore delete a file
that the current compaction never created.

Replacement allocation now skips up to 100 occupied candidates using
`symlink_metadata`. If snapshot construction fails before it successfully
publishes the selected replacement, active compaction performs no target-path
cleanup; its temporary-file guard still removes the file it actually owns. A
post-construction publication failure may remove the known-owned replacement,
while an ambiguous snapshot-publication error may conservatively leave a
complete orphan for operator inspection.

The isolated regression test relies on the deterministic process-local
sequence: initial database creation consumes sequence zero, then the test
places a sentinel at the first active-compaction candidate. Compaction skips
that path, publishes through the next candidate, preserves the sentinel bytes,
and leaves the reopened database fully verified with its committed value.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak bodies were
not enabled. No reboot, shutdown, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --all-features --test replacement_collision
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test maintenance_failures
  11 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test maintenance_crash
  4 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test compaction_race
  5 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test operations
  10 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  227 tests passed; 0 failed

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

The persistent database/WAL encodings and public Rust API are unchanged. This
closes a local destructive maintenance edge case; it does not close the
external Linux, physical power-loss, or pinned RocksDB release gates.
