# Ordered write-buffer node preflight — 2026-10-05

Source commit under test:
`1f992085e969a412e75befe738a661d13dbc46ae`
(`fix: preflight ordered buffer nodes`).

## Risk and contract

A buffered transaction becomes durable when its WAL append is synchronized.
The version/value arenas, point-update staging, hash-index slots, and shared key
storage were already prepared before that boundary, but a key first observed in
the current WAL generation still allocated a new ordered `BTreeMap` node during
in-memory publication. An allocator abort in that interval could therefore
leave a durable transaction recoverable from WAL but not published by the live
handle.

The ordered version map now has an internal prepared state:

- `None` means that the key's ordered-map node exists but has no published
  version; `Some(VersionId)` retains the previous MVCC meaning;
- value-arena reservation succeeds before any prepared node is inserted, and
  missing nodes are inserted while the write-buffer write lock is held and
  before WAL encoding or synchronization;
- point reads, ordered scans, ranges, verification, maintenance snapshots,
  membership-filter rebuilds, and lazy point-index construction all ignore
  prepared nodes, so readers continue observing the old immutable head while a
  synchronization is in flight;
- a successful durable apply changes an occupied node from `None` to
  `Some(VersionId)`. On the valid preflighted path this performs no ordered-map
  node or key allocation after synchronization;
- a failed or outcome-unknown WAL append removes every still-unpublished node
  before returning. A complete WAL tail remains authoritative and is recovered
  through the existing refresh protocol;
- incremental replay uses the same preparation rule before mutating its live
  buffer, and a dedicated `versioned_keys` count prevents prepared nodes from
  changing capacity calculations or logical sizing;
- main-file bytes, WAL bytes, transaction ordering, public API, and snapshot
  semantics are unchanged.

The causal unit test proves that a prepared new key physically occupies one
ordered node while remaining absent from point reads, scans, ranges,
maintenance enumeration, the point index, and the membership filter. It then
proves durable publication does not change the node count and reuses the same
reference-counted key. A second unit test covers explicit rollback. Injected
partial-write tests additionally retry both a single buffered commit and a
group commit on the same live handle using previously new keys.

## Verification

Cargo was bounded to two jobs for the release pass. Benchmark and opt-in soak
bodies were not enabled. No reboot, shutdown, sleep, power cut, raw-device
operation, physical cache reset, or benchmark run was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  245 Rust tests and 5 opt-in benchmark entrypoint checks passed; 0 failed

cargo test --features fault-injection --test io_failures
  7 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --lib \
  infrastructure::write_buffer::tests::prepared_ordered_nodes_are_invisible_and_reused_at_publication \
  --all-features
  1 passed; 0 failed

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

The final causal assertion expansion was test-only and was rerun after the full
suite; production sources were unchanged between those runs. This is a
crash-safety and allocation-boundary improvement, not a measured latency or
throughput claim. Linux release validation, physical power-loss validation,
cold-cache measurements, and the complete pinned RocksDB workload matrix
remain open release gates.
