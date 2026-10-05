# Automatic group-leader unwind verification — 2026-10-05

Source commit under test:
`181e83668e670f92d3af72d2955011c1ffa402be`
(`fix: fail group coordinator closed after unwind`).

## Risk and change

The automatic group-commit coordinator marks one caller as the active
foreground leader. Previously, normal success and typed errors explicitly
handed leadership to the next queued request, but an unexpected internal panic
between group capture and that call could unwind past the handoff. The
coordinator would retain `leader_active = true`; current request channels would
close, while later callers could wait forever because none could become leader.

Leadership is now owned by an RAII guard created before the writer lock or
queue is touched. Normal completion still transfers leadership to the next
live queue member. During unwind, however, the guard marks the coordinator
permanently failed, clears its active-leader state, and completes every queued
request with `Error::LockPoisoned`. Future submissions fail immediately with
the same typed error and require a database reopen. This is necessary because
a panic could occur after a durable WAL sync but before the in-memory append
boundary is advanced; continuing from that handle would not be safe.

A deterministic `catch_unwind` unit test owns one active group, queues a second
request, panics under the guard, and proves that the abandoned group, queued
request, and a future submission all return the typed synchronization error
without hanging.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
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

This is defensive fail-closed recovery after an implementation panic, not a
promise that such a panic is an accepted normal outcome or that the same handle
can safely continue. Persistent bytes and durability ordering are unchanged.
External Linux, filesystem power-loss, and pinned RocksDB release gates remain
open.
