# Automatic group unknown-outcome recovery — 2026-10-05

Source commit under test:
`453f951af8c19c454d64d41fa97e1f23ed177945`
(`fix: gate automatic batches after unknown commit`).

## Risk and contract

An automatic group whose WAL synchronization cannot be confirmed returns
`Error::CommitOutcomeUnknown` to every member. The durable bytes may contain
the complete group while the public `Database` head and in-memory WAL boundary
still describe the earlier snapshot. Passing leadership to a queued group in
that state would at best produce a stale-boundary conflict and could obscure
the required recovery decision.

The automatic coordinator now fails closed before releasing the writer mutex
when a group returns an unknown publication outcome, poisoned lock, or internal
invariant error. Queued and future `commit_batch` calls receive
`Error::LockPoisoned` without entering the storage engine. A successful
`Database::refresh` first installs the recovered durable generation and head,
then resets a quiescent failed coordinator. Reopening constructs a fresh one.

The fault-injection test fails the WAL sync for a four-member automatic group,
requires every member to receive `CommitOutcomeUnknown`, proves a subsequent
batch is rejected by the coordinator, refreshes to transaction 4, and then
successfully commits transaction 5. Independent reopen verifies four entries
at transaction 5; the fifth transaction is intentionally an empty batch.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
CARGO_BUILD_JOBS=2 cargo test --features fault-injection --test group_commit \
  automatic_group_failures_reach_every_waiter_and_recover
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --lib application::group_commit::tests
  4 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  218 tests passed; 0 failed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

The persistent format and WAL force ordering are unchanged. This closes the
same-handle automatic-batch retry gap, not the external Linux, filesystem
power-loss, or pinned RocksDB release gates.
