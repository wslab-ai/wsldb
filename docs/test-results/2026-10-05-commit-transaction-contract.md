# Commit transaction-ID contract

Date: 2026-10-05

## Risk

The application layer precomputed the transaction IDs promised to callers but
previously installed the head returned by the internal storage adapter without
checking that it ended at the same ID. The file adapter generated the expected
sequence, but an internal regression could have made the in-memory head and a
single or grouped commit result contradict the public ordering contract.

## Contract

`CommittedStorage` now has one authoritative transaction-ID validator. A
single commit requires exactly the successor of its base transaction. A group
requires the final consecutive ID precomputed for its exact batch count. The
check runs after the storage call but before either a replacement store or its
head is installed. A mismatch returns `Error::InternalInvariant` and leaves the
live database snapshot unchanged. A real adapter that had already published a
different durable head would therefore reject the stale handle's next write;
the application never exposes the contradictory head as successful state.

Focused adapter-seam tests return deliberately inconsistent heads for both
paths and prove that the database retains transaction ID zero.

## Verification

All builds used at most two Cargo jobs and a private temporary target directory
that was removed after completion.

```text
cargo fmt --all -- --check
  passed

cargo test --lib application::database::tests
  5 passed

cargo check --all-targets --all-features
  passed

cargo clippy --all-targets --all-features -- -D warnings
  passed

cargo test --all-targets --all-features
  101 library tests passed
  150 integration tests passed
  2 power-loss probe tests passed
  5 benchmark entrypoints remained opt-in
```

The complete executed total was 253 Rust tests. No benchmark workload, cache
reset, reboot, shutdown, sleep, or power operation ran.
