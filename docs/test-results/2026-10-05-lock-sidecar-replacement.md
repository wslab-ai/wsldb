# Lock-sidecar replacement verification — 2026-10-05

Source commit under test:
`e4b68bbd41d5986ffbc9220ba2cee1b5249a827b`
(`fix: detect replaced writer lock sidecars`).

## Risk and contract

An open Unix descriptor detects deletion when its link count becomes zero, but
renaming the stable `<database>.lock` pathname leaves the old inode linked. A
later open can then create and lock a new inode while the old handle continues
locking the renamed inode. Without a pathname-identity check, cooperating
writers could enter separate serialization domains.

The lock file is now a small typed resource that retains both its descriptor
and canonical sidecar path across engine replacement and background checkpoint
work. After every successful OS-lock acquisition, WSLDB validates that the
descriptor is still a regular, singly linked file and matches the device and
inode at the current path. A missing, renamed, replaced, symbolic, or
multiply-linked sidecar fails before protected database I/O and requires the
application to reopen the database. The failed guard releases the old OS lock.

Windows remains best-effort and is outside the Unix device/inode identity
contract. The check protects cooperating WSLDB processes; it does not claim to
make a filesystem namespace immutable against a continuously racing hostile
process.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::path_lock
  2 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test path_identity
  7 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  216 tests passed; 0 failed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

The persistent database/WAL encodings, durability ordering, and public Rust
API are unchanged. This does not close the external Linux, filesystem
power-loss, or pinned RocksDB release gates.
