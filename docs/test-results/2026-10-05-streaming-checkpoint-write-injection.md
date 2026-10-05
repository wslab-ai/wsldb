# Streaming-checkpoint write injection

Date: 2026-10-05

## Finding

The first-tree checkpoint fast path writes already ordered pages through a
256 KiB streaming buffer. Ordinary copy-on-write checkpoint pages had a
process-crash boundary after each positioned page write, but this streaming
write call had no path-specific crash or I/O-failure stage before its data
sync. Its pages are unpublished and the WAL remains authoritative, but that
recovery invariant was indirect rather than exercised at the write boundary.

## Change

Every streaming-buffer flush now exposes `checkpoint_stream_write`: injected
I/O failure occurs before the positioned write, while process-crash injection
occurs immediately after a complete positioned write and before the later data
sync. The source-scanning injection contract requires explicit crash and fault
test owners for the stage.

The boundary is diagnostic and test-only in normal builds. It changes neither
the persisted bytes nor the ordering of the production write and synchronization
operations.

## Recovery coverage

The crash subprocess starts from a valid empty page tree, durably commits 64
keys to the WAL, and aborts at the first streaming checkpoint write. Reopen
recovers the exact 64-key model and full verification succeeds.

The fault subprocess injects an error before the same write. The background
checkpoint records one failure, the acknowledged mutation remains visible,
and reopen/verification continue to use the durable WAL state.

## Verification

All Cargo commands used at most two jobs and a private temporary target
directory that was deleted after completion.

```text
cargo test --all-features --test buffer_checkpoint_crash \
  interrupted_buffer_checkpoints_recover_the_durable_commit -- --exact
  1 passed

cargo test --all-features --test maintenance_failures \
  automatic_checkpoint_failures_keep_the_wal_commit_durable -- --exact
  1 passed

cargo test --all-features --test injection_contract
  2 passed

cargo test --all-targets --all-features
  109 library tests passed
  159 integration tests passed
  2 power-loss probe tests passed
  5 benchmark entrypoints remained opt-in

cargo check --all-targets --all-features
  passed

cargo clippy --all-targets --all-features -- -D warnings
  passed

RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
  passed

cargo fmt --all -- --check
  passed

git diff --check
  passed

scripts/release-audit.sh
  passed
```

The complete executed suite contained 270 Rust tests. No benchmark workload,
opt-in soak body, corruption-fuzz body, cache reset, reboot, shutdown, sleep,
raw-device, or power operation ran.
