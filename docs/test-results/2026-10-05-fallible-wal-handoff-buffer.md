# Fallible WAL-handoff copy buffer

Date: 2026-10-05

## Risk

Buffered active compaction copies the authoritative WAL into a staged inode
before replacing the main-file generation. Its fixed 256 KiB transfer buffer
was previously created with an infallible `vec![]` allocation. Under memory
pressure, that allocation could abort the process instead of returning through
the database error contract, even though publication had not begun.

## Change

The allocation module now owns one `zeroed_bytes` helper that reserves capacity
with `Vec::try_reserve_exact` before initialization. WAL decoding and the
active-compaction handoff both use this implementation. If the handoff buffer
cannot be reserved, staging returns `Error::FileTooLarge` before reading or
writing handoff bytes; the source main file and WAL remain authoritative.

## Causal coverage

A unit test requests an impossible `usize::MAX` zeroed buffer and requires the
typed error. The existing WAL record allocation boundary still exercises the
same helper through the decoder, while the all-feature active-compaction,
crash, and injected-I/O suites cover the successful handoff lifecycle.

## Verification

All Cargo commands used at most two jobs and private temporary target
directories that were deleted after completion.

```text
cargo test --all-features \
  allocation::tests::zeroed_allocation_failure_is_typed
  1 passed

cargo test --all-targets --all-features
  108 library tests passed
  156 integration tests passed
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

The complete executed suite contained 266 Rust tests. No benchmark workload,
cache reset, reboot, shutdown, sleep, raw-device, or power operation ran.
