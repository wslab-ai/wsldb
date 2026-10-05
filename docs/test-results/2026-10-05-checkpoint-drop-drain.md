# Checkpoint drain on storage-generation drop

Date: 2026-10-05

## Finding

Dropping Rust's `JoinHandle` detaches its thread. `CheckpointTask` previously
had no destructor, so destroying the final owner of a buffered storage
generation while checkpoint preparation was active could return while that
worker continued writing and synchronizing unreachable pages. The durable WAL
remained authoritative, but the behavior violated the lifecycle expectation
that no hidden checkpoint I/O survives destruction of its final owner.

## Change

`CheckpointTask::drop` now consumes its private worker state and joins a
running worker. It does not publish prepared metadata or change the logical
head; completed but unpublished pages remain unreachable and ordinary
compaction can reclaim them. Worker errors cannot be returned from a Rust
destructor and are discarded only after the worker has terminated. Existing
explicit `flush` and active-compaction paths continue to collect and handle the
worker result before destruction.

The `DatabaseOptions::write_buffer_bytes` contract and architecture document
now state that destruction of the final buffered-generation owner may wait for
already-started checkpoint work and does not leave detached checkpoint I/O.

## Deterministic regression

The all-feature subprocess test pauses a real checkpoint worker immediately
after its data sync. It starts destruction of the only database handle and
proves the process remains alive while the worker is paused. After releasing
the pausepoint, destruction completes; reopening without buffered options
recovers the acknowledged value from the WAL and full verification succeeds.

## Verification

All Cargo commands used at most two jobs and a private temporary target
directory that was deleted after completion.

```text
cargo test --all-features --test freeze_swap_checkpoint \
  dropping_the_last_handle_waits_for_a_pending_checkpoint_worker -- --exact
  1 passed

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
