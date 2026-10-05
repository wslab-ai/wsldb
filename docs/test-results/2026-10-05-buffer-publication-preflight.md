# Durable-buffer publication preflight — 2026-10-05

Source commit under test:
`cd74fad13a4d1214a3f90c9e83a007831a75bc6e`
(`fix: preflight durable buffer publication`).

## Risk and contract

A buffered commit becomes durable when its WAL append is synchronized. The
following in-memory publication previously grew the contiguous version arena,
value arena, point-update vector, and 64 temporary shard vectors. A large
transaction or group could therefore encounter an avoidable allocation failure
after durable state had already advanced.

The commit path now:

- counts mutations and value bytes with checked arithmetic and fallibly
  reserves both arena vectors before the WAL write;
- reserves transaction- or group-sized point-update staging before the WAL
  write, including the race in which the lazy point index is first constructed
  while synchronization is in flight;
- validates an existing point index and fallibly reserves its new per-shard
  hash slots during preflight;
- groups staged point updates by shard in place, avoiding 64 allocation-capable
  temporary vectors after synchronization;
- applies the same arena and update-staging preflight before incremental replay
  mutates an in-memory buffer.

Focused capacity tests prove that applying prepared records does not grow the
version/value arenas and that inserting a preflighted new point-index key does
not grow its shard capacity. Existing lazy-index, group, refresh, checkpoint,
crash, and recovery tests cover the surrounding publication protocol.

This is not a claim that durable publication is completely allocation-free.
New ordered-map nodes and reference-counted keys can still require allocation.
The point-index initialization race described by this original evidence was
subsequently closed by a guarded fallible builder
([successor evidence](2026-10-05-fallible-lazy-point-index.md)); the ordered-map
path was subsequently moved before WAL synchronization as a separate change
([successor evidence](2026-10-05-ordered-buffer-node-preflight.md)).

## Verification

Cargo was bounded to two jobs. Benchmark and opt-in soak bodies were not
enabled. No reboot, shutdown, sleep, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::write_buffer::tests \
  --all-features
  10 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test write_buffer --all-features
  16 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test group_commit --all-features
  12 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test refresh --all-features
  5 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test lazy_point_index --all-features
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test freeze_swap_checkpoint --all-features
  3 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test write_buffer_verification --all-features
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  241 Rust tests and 5 opt-in benchmark entrypoint checks passed; 0 failed

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
performance comparison was run, so this change supplies no new RocksDB claim.
Linux, physical power-loss, cold-cache, and pinned RocksDB release gates remain
open.
