# Lock-sidecar unlink verification — 2026-10-05

Source commit under test:
`816dc97962f09221ec81577f4a450627b46df595`.

## Risk and contract

WSLDB retains an open descriptor for the stable `<database>.lock` sidecar. If
an operator deleted that pathname while handles were alive, the old descriptor
could still lock an unlinked inode while a newly opened handle created and
locked a different inode. Cooperating writers would then no longer share one
serialization domain.

On Unix, `AdvisoryFileLock::acquire` now checks the descriptor link count after
the OS lock succeeds and before any protected database I/O. An unlinked
descriptor returns `Error::InvalidInput` with an explicit reopen instruction;
the guard is dropped and unlocks the stale inode. A fresh `Database::open`
creates the current sidecar and can continue from the unchanged durable state.

The `.lock` file remains internal live state and must not be manually deleted
or renamed. This check closes deletion that happened before acquisition; it
does not claim protection from an uncooperative external process racing a
sidecar mutation during an already active operation. Windows remains
best-effort and does not participate in this Unix release contract.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::path_lock
  2 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test path_identity
  4 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  213 tests passed; 0 failed

scripts/release-audit.sh
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

The persistent database/WAL encodings and public Rust API are unchanged. This
does not close the external Linux, filesystem power-loss, or pinned RocksDB
release gates.
