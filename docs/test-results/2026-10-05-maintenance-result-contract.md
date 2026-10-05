# Maintenance result contract

Date: 2026-10-05

## Risk

The storage engine fully verifies its flush and compaction products, but the
application layer previously installed the returned store and head without
checking that the result still described the requested logical snapshot. A
future adapter regression could therefore make a maintenance operation appear
successful while publishing contradictory in-memory state.

## Contract

The application/storage boundary now validates maintenance results before
acquiring the state write lock or installing either the returned store or its
head:

- flush may change physical root and page allocation, but must preserve the
  transaction ID and entry count;
- active compaction must report the exact source transaction ID it received;
- the compaction report's already verified snapshot transaction ID and entry
  count must match the returned head.

These are constant-time comparisons over data the operations already produce;
they do not add another tree scan. Adapter-seam tests deliberately return an
invalid flush head and a compaction head/report disagreement. Both operations
return `Error::InternalInvariant`, and the live database statistics remain on
the original snapshot. Existing direct and buffered integration tests exercise
valid flush and compaction results through the same checks.

## Verification

All builds used at most two Cargo jobs in private temporary target directories
that were deleted after completion.

```text
cargo fmt --all -- --check
  passed

cargo test --lib application::database::tests
  9 passed

cargo check --all-targets --all-features
  passed

cargo clippy --all-targets --all-features -- -D warnings
  passed

cargo test --all-targets --all-features
  105 library tests passed
  150 integration tests passed
  2 power-loss probe tests passed
  5 benchmark entrypoints remained opt-in

git diff --check
  passed
```

The complete executed total was 257 Rust tests. No benchmark workload, cache
reset, reboot, shutdown, sleep, or power operation ran.
