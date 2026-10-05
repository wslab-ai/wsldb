# Direct-mode WAL recovery verification — 2026-10-05

Source commit under test:
`92f97a3039863ac0dda544716c0cd79fc61eaa2a`
(`fix: honor direct mode after wal recovery`).

## Risk and contract

Opening a database with `DatabaseOptions::write_buffer_bytes(0)` must select
the direct copy-on-write commit path. An existing nonempty WAL still has to be
replayed so acknowledged buffered commits remain visible. The previous open
path handled that recovery by replacing the requested zero threshold with 64
MiB, which silently left the handle in buffered mode indefinitely. It also
made explicit and automatic group commit available even though their public
contract requires buffering to be enabled.

The configured threshold now remains zero. While recovered WAL state exists,
ordinary reads and writes continue safely through a temporary buffer with a
bounded internal checkpoint threshold. Group commit remains rejected. An
explicit flush bypasses WAL-snapshot consolidation, publishes a page-tree
checkpoint, clears the WAL, and installs a direct storage engine. Automatic
checkpoint completion makes the same transition when there is no suffix; a
concurrent suffix remains recoverable and is materialized by a later
checkpoint.

The focused integration test creates a buffered WAL, reopens with default
options, verifies recovery, proves group commit is disabled, flushes, and then
proves the next commit writes data pages without adding WAL bytes. A final
independent reopen verifies both transactions. Existing WAL-snapshot crash
coverage now opens with buffering explicitly and remains mapped to all four
snapshot publication failpoints. The persisted format is unchanged.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
CARGO_BUILD_JOBS=2 cargo test --all-features --test write_buffer_mode
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test write_buffer_snapshot_crash
  2 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test injection_contract
  2 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  224 tests passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" CARGO_BUILD_JOBS=2 cargo doc --no-deps --all-features
  passed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

This closes a runtime-options contract violation and preserves recovery of
previously acknowledged commits. It does not close the external Linux,
filesystem power-loss, or pinned RocksDB release gates.
