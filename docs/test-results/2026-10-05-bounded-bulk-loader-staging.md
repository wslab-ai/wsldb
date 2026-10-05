# Bounded bulk-loader staging verification — 2026-10-05

Source commit under test:
`bc1e4bde5b9b1118dda55efef306af8834a29ec0`
(`fix: bound bulk loader staging`).

## Risk and contract

The bottom-up bulk loader writes ordered snapshots for initial-tree creation,
buffer checkpoints, and compaction. With a streaming or direct writer,
overflow and leaf pages can already be written while the loader is still
building its leaf index and upper branch levels. Several vectors previously
grew through ordinary `push` or `collect`, including the database-sized leaf
level and each successive branch level.

The loader now obtains fallible capacity before accepting a leaf entry. It
copies the bounded key and validates the next logical entry count before
`store_value` can stage or write overflow pages. Before writing a completed
leaf, it reserves the leaf-level child slot and fallibly copies the minimum
key. Re-inserting the entry that caused a leaf split is also preceded by a
fallible capacity check.

Every branch candidate now builds its page-ID and separator vectors with exact
fallible reservations and fallible minimum-key copies. A next-level output slot
and its propagated minimum are ready before the branch page is written. The
bounded maintenance chunk similarly reserves its entry slot before accepting
ownership, and the direct compaction writer validates the next page ID before
issuing the current page write.

Ordering, leaf and branch partition rules, page encoding, checksums,
publication order, file format, and public API are unchanged. Key boxes and
bounded node clones still rely on standard Rust allocation; this is not a
claim of universal recovery from system-wide out-of-memory conditions. Any
error after writes to an unpublished checkpoint or compaction destination
continues to leave the prior database snapshot authoritative.

## Verification

Cargo work was limited to two build jobs. Benchmark, soak, physical power-loss,
shutdown, and reboot modes were not invoked.

```text
CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::bulk::tests --all-features
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test operations --all-features
  10 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  229 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" CARGO_BUILD_JOBS=2 cargo doc --no-deps --all-features
cargo fmt --all -- --check
git diff --check
  passed

CARGO_BUILD_JOBS=2 scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed
```

This closes the identified database-sized infallible vector growth in bulk
tree construction and its maintenance input chunk. External Linux, filesystem
power-loss, cold-cache, and pinned RocksDB release gates remain open.
