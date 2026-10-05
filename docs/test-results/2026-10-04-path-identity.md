# Database path identity verification — 2026-10-04

Source commit under test:
`35298fa3204f920b76316644cce2259a46f89bb8`.

## Risk and contract

The stable sidecar lock used the caller-provided database path. Before this
change, a symbolic-link alias could select a different lock and WAL name for
the same main-file inode. A relative path could also change meaning if the
process working directory changed after open. Distinct hard links cannot be
made to share one pathname sidecar portably.

Opening now resolves an existing database to its canonical absolute path and
resolves the parent of a new database before selecting any sidecar. On Unix,
an open fails closed when the database has unrelated hard links. Recovery first
removes only reserved initialization-temporary names whose device and inode
exactly match the published database, then syncs the parent directory. This
preserves recovery after a crash between no-clobber link publication and
temporary-name removal.

Focused tests prove that a symbolic-link open writes the canonical `.lock` and
`.wal`, an unrelated hard-link alias is rejected, and a same-inode published
temporary link is removed before full verification. Existing initialization
tests cover every creation crash and injected-I/O boundary.

## Verification

The laptop run was bounded to two Cargo build jobs. No benchmark, soak mode,
reboot, shutdown, power cut, or raw-device operation was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --test path_identity
  3 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test initialization \
  --features crash-injection,fault-injection
  7 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  210 tests passed; 0 failed

RUSTDOCFLAGS="-D warnings" CARGO_BUILD_JOBS=2 \
  cargo doc --no-deps --all-features
  passed

scripts/release-audit.sh
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

The persistent main-file and WAL encodings and the public Rust API contract are
unchanged. This evidence closes the local path-alias serialization defect; it
does not close the external Linux, filesystem power-loss, or pinned RocksDB
release gates.
