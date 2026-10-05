# Buffered main-file identity verification — 2026-10-05

Source commit under test:
`356c8add055a32fef485999bf5f82c7e0320ba22`
(`fix: reject replaced main files across operations`), including parents
`d837180682570bc826c8ee15a7501916b5110f50` and
`ac34e9895451695f1339f54597bda6775ae86312`.

## Risk and contract

The accepted non-empty-WAL commit fast path avoided reopening and decoding the
main file on every buffered commit. Its original argument covered changes made
through cooperating WSLDB operations, but it did not cover an external actor
atomically replacing the main path with a byte-identical file. The buffered
handle could retain the unlinked old descriptor while the canonical path named
a different inode with the same `Head`.

On Unix, both empty- and non-empty-WAL commit paths now compare the device and
inode at the canonical main path with the descriptor pinned by the active
storage generation while holding the stable interprocess writer lock. A
missing, non-regular, multiply linked, or different current file returns
`WriteConflict` before the WAL is appended. Buffered refresh performs the same
generation comparison; a mismatch bypasses incremental tail refresh and
reopens the complete current generation.

The same fail-closed identity contract now covers direct and buffered flush,
buffered operational statistics, backup and compaction snapshot capture,
checkpoint publication, and active compaction publication. Standalone
`Database::compact_file()` also compares its captured source descriptor with
the current path immediately before replacing the database. A different inode
with an equal `Head` can therefore neither receive prepared checkpoint pages
nor be overwritten by a compaction built from an older generation.

The regression test first leaves a durable transaction in a non-empty WAL,
records the WAL length, and atomically installs a byte-identical main-file
copy. It proves that the stale commit returns `WriteConflict` without changing
the WAL length. `Database::refresh()` then preserves the earlier durable
transaction, switches the same handle to the replacement generation, permits
a new commit, and keeps the rejected mutation absent.

A second regression covers replacement before the first buffered transaction
and proves that the empty WAL remains empty. Main-dependent operations reject
the stale generation without creating backup or compaction targets. A
pausepoint-driven process race replaces the main inode after direct and
buffered compaction output is ready but before publication; both modes return
`WriteConflict`, preserve the complete logical snapshot and WAL, and leave no
replacement or handoff files.

Windows remains best effort and outside this Unix device/inode contract until
an audited stable file-ID adapter is available.

## Verification

Cargo was bounded to two jobs. Benchmark and opt-in soak bodies were not
enabled. No reboot, shutdown, sleep, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --test path_identity --all-features
  11 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test compaction_race --all-features
  6 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test operations --all-features
  10 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test refresh --all-features
  5 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test write_buffer --all-features
  16 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  238 tests passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" CARGO_BUILD_JOBS=2 \
  cargo doc --no-deps --all-features
  passed

CARGO_BUILD_JOBS=2 scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

The public Rust API and persisted main-file and WAL encodings are unchanged.
The additional Unix metadata checks are correctness requirements. The pinned
W5/W6 RocksDB comparison must be rerun on this head before using earlier
throughput or latency results; this evidence makes no performance-parity or
superiority claim. External Linux and physical power-loss release gates also
remain open.
