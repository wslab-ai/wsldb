# Fallible large-value copy verification — 2026-10-05

Source commit under test:
`4dba8c02941a3df48e045457052298cb4d180ccb`
(`fix: make large value copies fallible`).

## Risk and contract

WSLDB accepts values up to 64 MiB. Several paths copied those values with
`slice::to_vec`: public `WriteBatch::put`, transactional `put` and
read-your-writes, ordinary and compact-group WAL decoding, buffered point
reads, range merging, and maintenance overlay construction. Those copies used
the infallible allocator path and could abort the process under memory pressure
instead of returning a typed database error.

A shared private `clone_bytes` operation now reserves the exact contiguous
capacity with `Vec::try_reserve_exact` before copying. Allocation refusal is
reported as `Error::FileTooLarge` before public mutation state changes, before
a decoded value enters recovery state, or before a buffered read result is
returned. Page-local inline values remain bounded to 2 KiB and are outside this
large-allocation change.

The persistent bytes are unchanged: this is allocation policy around existing
value copies, not a main-file or WAL codec change. Existing overflow-value,
buffered-recovery, compact-group, format-fixture, crash, corruption, and
maintenance tests exercise the modified paths.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak bodies were
not enabled. No reboot, shutdown, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --all-features --test database \
  overflow_value_survives_reopen_and_is_checksummed
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test write_buffer \
  buffered_commits_preserve_snapshots_and_recover_without_options
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test group_commit \
  ordered_batches_share_one_durable_sync_and_recover
  1 passed; 0 failed

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

This closes the known infallible 64 MiB value-copy paths. It does not claim
that Rust collection metadata allocations are universally recoverable, and it
does not close the external Linux, physical power-loss, or pinned RocksDB
release gates.
