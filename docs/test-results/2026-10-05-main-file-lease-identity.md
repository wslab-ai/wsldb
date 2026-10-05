# Main-file lease identity verification — 2026-10-05

Source commit under test:
`a20341071bf8cdec3f7a9b433b75bc64f9bff5f4`
(`fix: validate main file leases after locking`).

## Risk and contract

Active compaction and buffered checkpoint publication use an advisory lock on
the main-file inode as a long-lived generation lease. Previously the blocking
path opened the database and then waited for that inode's lock without checking
the pathname again. If another process compacted and atomically replaced the
main path while the waiter was blocked, the waiter could eventually acquire a
lock on the now-unlinked old inode. A later checkpoint or compaction would then
mistakenly treat that lock as exclusion for the current generation.

Both blocking `acquire` and non-blocking `try_acquire` now reopen the canonical
main path after the OS lock succeeds and compare its Unix device/inode with the
locked descriptor. A mismatch returns `WriteConflict`; dropping the failed
lease releases the stale lock. Once the comparison succeeds, every cooperating
WSLDB replacement must acquire the same locked inode, so it cannot pass the
lease until the holder finishes.

The deterministic regression test opens a descriptor, atomically replaces its
path with a new inode, and passes that pre-replacement descriptor through the
same blocking acquisition implementation. The post-lock identity validation
rejects it. Existing tests then exercise normal exclusion, active compaction,
buffered checkpointing, direct and buffered publication races, and WAL
handoff recovery.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak bodies were
not enabled. No reboot, shutdown, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --all-features --lib \
  infrastructure::path_lock::tests
  3 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test operations \
  atomic_compaction_preserves_old_readers_and_rejects_stale_writers
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test write_buffer \
  automatic_buffer_checkpoints_preserve_old_readers
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-features --test compaction_race
  5 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  226 tests passed; 0 failed

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

The persistent database/WAL encodings and public Rust API are unchanged. This
closes a local cross-process generation-lease race; it does not close the
external Linux, physical power-loss, or pinned RocksDB release gates.
