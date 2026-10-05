# Fallible point-index preflight verification — 2026-10-05

Source commit under test:
`a14663356b8c75675ecf80c7604eb995b736ca01`
(`fix: make point index preflight fallible`).

## Risk and contract

When point reads have materialized the lazy sharded index, buffered commit and
incremental refresh validate its key-presence view before applying new WAL
records. That validation previously collected a mutation-sized presence vector
and per-shard key vectors with ordinary collection growth. Allocation failure
could therefore terminate the process during a phase intended to fail before a
new WAL append.

The 64 point-index shards are now represented by a fixed array rather than a
heap-allocated outer vector. Generic replay preflight reserves the complete
ordered-presence vector fallibly. Known-presence preflight uses a fixed array
of shard buckets and obtains fallible capacity before placing each mutation in
its bucket. A capacity failure is returned as `Error::FileTooLarge` before the
caller enters WAL encoding or durable append.

The same fixed outer array is used when publishing already durable point-index
updates, removing one bounded heap allocation from that phase. Inner index
maps, key ownership, and version arenas still use standard Rust collections;
this change does not claim process-level recovery from every possible
system-wide out-of-memory condition. A process loss after a successful WAL
sync remains recoverable from the durable WAL.

Shard selection, transaction visibility, lazy initialization, WAL bytes, file
format, and public API are unchanged.

## Verification

Cargo work was limited to two build jobs. Benchmark, soak, physical power-loss,
shutdown, and reboot modes were not invoked.

```text
CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::write_buffer::tests --all-features
  7 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test lazy_point_index --all-features
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test refresh --all-features
  5 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  228 passed; 0 failed

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

This closes the identified mutation-sized infallible allocations in point-index
preflight. External Linux, filesystem power-loss, cold-cache, and pinned
RocksDB release gates remain open.
