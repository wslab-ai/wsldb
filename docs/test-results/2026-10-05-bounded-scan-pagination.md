# Bounded ordered-scan pagination

Date: 2026-10-05

## Risk

`ReadTransaction::range` bounded one materialized result, but its inclusive
start key was awkward for pagination: callers had to construct a bytewise
successor or fetch and discard the previous boundary. The latter can needlessly
read and copy a value as large as 64 MiB. `iter` still materializes the complete
snapshot and is not appropriate for memory-bounded traversal of a large
database.

## Contract

`ReadTransaction::range_after(after, end, limit)` returns at most `limit`
entries strictly after `after`; `None` begins at the first key and `end` remains
exclusive. Reusing the last returned key on the same immutable read transaction
therefore produces duplicate-free, gap-free pages without constructing a key
successor.

The exclusive bound is carried through B+ tree branch pruning, ordinary and
packed leaves, the active write-buffer overlay, and a frozen checkpoint overlay.
It is not implemented as an inclusive read followed by a discarded entry. A
focused cursor test gives the boundary key an unresolved overflow chain and
proves a page after that key still succeeds without reading its value.

The cross-engine model property now reconstructs every historical and current
snapshot in seven-entry pages for direct storage, buffered storage, post-flush
state, and reopened state. It compares the concatenated result with the
independent ordered model and also checks an exclusive lower plus exclusive
upper bound.

The external-consumer contract includes the new method. Its reviewed SHA-256
is `2ae6abb818ee92d5b6af81c9169f6c24c7eb551ee64d5df0cec3a7d12cf81829`.
No persisted bytes or existing API semantics changed.

## Verification

Cargo work used two build jobs and a private temporary target directory. No
benchmark, soak, cache reset, shutdown, reboot, or power operation was run.

```text
cargo test --all-features --test model_properties --test public_api
  4 passed; 0 failed

cargo test --all-features --lib range
  4 passed; 0 failed

cargo test --all-features --lib exclusive_start_does_not_read_the_boundary_value
  1 passed; 0 failed

cargo test --all-targets --all-features
  274 passed; 0 failed

cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
git diff --check
scripts/release-audit.sh
  passed
```

This closes bounded public pagination, not streaming borrowed iteration. The
external Linux, filesystem power-loss, cold-cache, and pinned RocksDB release
gates remain open.
