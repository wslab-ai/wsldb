# Shared buffered mutation keys — 2026-10-05

Source commit under test:
`3d4ebf4b7b994e8234e8f6ae9fb5d424e080011d`
(`fix: reuse buffered mutation keys`).

## Risk and contract

`ChangeSet` previously stored keys as `Vec<u8>`. After the corresponding WAL
was synchronized, durable in-memory publication converted every consumed key
to `Arc<[u8]>` for the ordered write-buffer map and point index. That conversion
could allocate and copy transaction-sized key data after durable state had
already advanced.

Mutation keys are now reference-counted when their change set is formed or
decoded. The storage paths then preserve that ownership:

- durable write-buffer publication clones the existing reference for its
  ordered map and moves the same reference into point-update staging;
- a causal unit test uses `Arc::ptr_eq` to prove the change set, ordered map,
  and point update share one key allocation;
- write-buffer snapshots and layered maintenance overlays move or clone the
  shared key instead of copying it back into a vector;
- layered compaction distinguishes owned base-tree keys from shared overlay
  keys, retaining both forms until the bounded maintenance chunk is consumed;
- ordering, lookup semantics, logical byte accounting, WAL bytes, and persisted
  database bytes are unchanged.

This removes the per-key reference-counted allocation from post-sync
publication. It does not make publication allocation-free: a newly observed
key can still require a node allocation in the ordered `BTreeMap`. Closing that
path required a separate ordered-index design and was subsequently completed
with invisible pre-WAL nodes
([successor evidence](2026-10-05-ordered-buffer-node-preflight.md)).

## Verification

Cargo was bounded to two jobs. Benchmark and opt-in soak bodies were not
enabled. No reboot, shutdown, sleep, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --lib domain::change_set::tests --all-features
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::write_buffer::tests \
  --all-features
  12 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::wal::tests --all-features
  25 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test write_buffer --all-features
  16 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test operations --all-features
  10 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  243 Rust tests and 5 opt-in benchmark entrypoint checks passed; 0 failed

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
performance comparison was run, so fewer allocations are not presented as a
measured throughput improvement. Linux, physical power-loss, cold-cache, and
pinned RocksDB release gates remain open.
