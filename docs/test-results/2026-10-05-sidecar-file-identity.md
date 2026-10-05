# Persistent-file identity verification — 2026-10-05

Source commit under test:
`52119b647d84c22e3d616a24b0dd702011d5c6a7`
(`fix: validate persistent sidecar identities`).

## Risk and contract

A lock or WAL pathname that resolved through a symbolic link could redirect
WSLDB I/O into an unrelated file. A multiply hard-linked sidecar could also be
shared across database path identities or mutated through an unexpected name.
Non-regular main paths were not rejected with a WSLDB-specific error before
opening their sidecars.

The main database path must now resolve to a regular file. Existing lock and
WAL sidecars must be regular files and, on Unix, have exactly one directory
link. Their open descriptors are checked against the current pathname identity
before recovery or publication. A WAL descriptor unlinked by WSLDB's own
atomic replacement remains a normal stale descriptor and causes reopen of the
current WAL rather than an error.

The checks fail closed against pre-existing redirects and identity changes
during open. They do not claim to make filesystem namespaces tamper-proof
against an uncooperative process racing every validation. Windows remains
best-effort and is outside the Unix hard-link identity contract.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::wal
  23 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test path_identity
  6 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  215 tests passed; 0 failed

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
