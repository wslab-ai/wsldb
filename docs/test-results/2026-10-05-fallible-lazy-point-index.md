# Fallible lazy point index — 2026-10-05

Source commit under test:
`85ea44a5a06d521eff1c3a1af7951ae635ef0941`
(`fix: make lazy point index fallible`).

## Risk and contract

The first buffered point read materializes a 64-shard hash index from every key
currently represented in the WAL overlay. The old `OnceLock` initializer grew
each `HashMap` through infallible inserts. A large overlay could therefore abort
the process during an otherwise read-only operation. A first read could also
construct the old index while a commit's WAL synchronization was in flight,
forcing that commit to stage and publish point updates after synchronization
even when the workload had previously been write-only.

The lazy index now uses a dedicated synchronized slot:

- construction counts keys per shard with checked arithmetic and fallibly
  reserves every hash map before any key is inserted;
- an ordinary build error leaves the slot empty and retryable rather than
  publishing a partial index;
- concurrent first readers still publish exactly one complete index;
- the lock order is initialization guard followed by write-buffer state, so
  commit/read coordination does not invert those locks;
- a commit that observes an empty slot retains the initialization guard through
  WAL synchronization and in-memory publication. The first later reader then
  builds from the published head instead of a stale pre-commit buffer;
- write-only single and group commits no longer reserve unused point-update
  staging, while an already materialized index retains its fallible preflight
  and durable catch-up path;
- snapshot generations continue sharing the same immutable slot and arena.

The point-index code was split into its own 245-line module; the owning
write-buffer module is now 453 lines. Both remain below the repository's
700-line source limit.

This closes fallible lazy construction and the during-sync initialization
race. It does not make all post-sync publication allocation-free. Conversion
of new keys to reference-counted storage was subsequently moved before durable
publication ([successor evidence](2026-10-05-shared-buffer-keys.md)); insertion
of new ordered-map nodes was subsequently moved before WAL synchronization
([successor evidence](2026-10-05-ordered-buffer-node-preflight.md)).

## Verification

Cargo was bounded to two jobs. Benchmark and opt-in soak bodies were not
enabled. No reboot, shutdown, sleep, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::write_buffer::tests \
  --all-features
  11 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test lazy_point_index --all-features
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test snapshot_handoff --all-features
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test write_buffer --all-features
  16 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test group_commit --all-features
  12 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test refresh --all-features
  5 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  242 Rust tests and 5 opt-in benchmark entrypoint checks passed; 0 failed

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

The public API and persisted main-file and WAL encodings are unchanged. No
performance comparison was run, so the eliminated write-only staging is not a
measured throughput claim. Linux, physical power-loss, cold-cache, and pinned
RocksDB release gates remain open.
