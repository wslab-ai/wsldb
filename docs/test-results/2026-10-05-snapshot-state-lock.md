# Snapshot-operation state-lock verification — 2026-10-05

Source commit under test:
`da57d09e3fa535e4ea432d901726dd6d94307373`
(`perf: release database state during snapshot work`).

## Risk and change

The application layer previously retained the shared `DatabaseState` read lock
while `verify`, `backup`, `compact_to`, and `operational_stats` performed their
potentially long storage work. A commit could durably publish through the
storage engine but then wait for that read lock before updating the same public
`Database` handle and returning to its caller. This defeated the immutable-
snapshot concurrency already provided by the storage layer.

These operations now clone the current storage generation and capture its
`Head` under the state lock, then release the lock before entering the storage
operation. The `Arc` keeps the selected generation alive, and the captured
head keeps explicit verification pinned to one immutable snapshot. No
persistent format, durability order, or public type changed.

A deterministic unit seam blocks verification inside a test storage engine,
starts a commit through the same cloned `Database`, and requires that the
commit update the public handle before verification is released. The test
would time out under the previous lock lifetime and always releases its barrier
before asserting, so a regression cannot strand a test thread.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
CARGO_BUILD_JOBS=2 cargo test --lib \
  application::database::tests::long_snapshot_read_does_not_block_commit_publication
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  217 tests passed; 0 failed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

This proves application-state publication concurrency, not that every storage
backend phase is lock-free. Short storage commit/file locks still capture a
consistent backup, compaction, or statistics snapshot, and buffered
verification may serialize with write-buffer transitions. External Linux,
filesystem power-loss, and pinned RocksDB release gates remain open.
