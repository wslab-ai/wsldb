# Live WAL identity coverage

Source commit under test: `e58a141` (`test: cover live WAL replacement`).
Maintenance follow-up: `6560b20` (`test: cover WAL replacement maintenance`).

## Risk

WSLDB already validated WAL file type and link count while opening, and the
buffered engine compared its open WAL descriptor with the current sidecar
before append. Integration coverage proved live main-file and lock-sidecar
replacement behavior, but did not exercise replacement of a nonempty WAL while
a buffered handle remained open.

Without that causal test, a regression could append durable records to an
unlinked inode while a later opener recovered from a different `.wal` path.
Logical hashes on ordinary runs would not expose the split generation.

## Contract

The Unix integration test creates a nonempty durable WAL, copies it
byte-for-byte to a new inode, and atomically replaces the sidecar path. Before
any append, the stale handle must return `Error::WriteConflict` for:

- an ordinary `WriteTransaction` commit;
- an explicit `commit_group`;
- an automatic `commit_batch`.

The replacement WAL length must remain unchanged and all rejected keys must
remain absent. `Database::refresh` then reopens the byte-identical current WAL,
preserves the prior durable value, and permits a new commit. The existing main
replacement test now applies the same three commit-path assertions.
Before refresh, `operational_stats`, `backup`, `compact_to`, active `compact`,
and `flush` must also return `WriteConflict`; backup and compact outputs must
not be created. This prevents verification or maintenance from combining the
open WAL inode with the current sidecar generation.

## Verification

```text
cargo fmt --all -- --check
  passed

CARGO_TARGET_DIR=/tmp/wsldb-wal-identity.<temporary> \
  cargo test --test path_identity --all-features
  12 passed; 0 failed

CARGO_TARGET_DIR=/tmp/wsldb-wal-identity.<temporary> \
  cargo clippy --test path_identity --all-features -- -D warnings
  passed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

git diff --check
  passed
```

The private Cargo target was removed after verification, and the repository
has no `target` directory. This is deterministic Unix identity coverage, not a
physical power-loss result. No benchmark, cache reset, reboot, shutdown, sleep,
raw-device operation, or power-removal test was run on the development laptop.
