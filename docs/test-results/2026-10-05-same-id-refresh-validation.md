# Same-ID in-store refresh validation

Date: 2026-10-05

## Risk

`Database::refresh` already fully compared a replacement storage generation
whose transaction ID equalled the live snapshot. A storage adapter could also
return a different head on the current store without returning a replacement
store, as happens conceptually when maintenance publishes a new physical head
for unchanged logical state. That path previously installed the head without
the documented logical-equivalence check.

The file adapter's normal paths preserve the invariant, but the application
boundary could therefore accept a future adapter regression or malformed
same-ID maintenance result whose entry count or logical contents differed from
the live snapshot.

## Contract

When refresh returns the same transaction ID, the exact unchanged head keeps
the existing no-scan fast path. Any different head is fully verified against
the live head even when both use the same storage object. Entry count and the
canonical logical hash must match before the candidate is installed. A
replacement storage object continues to require the comparison even when its
head fields are byte-for-byte equal.

Adapter-seam tests prove both sides: a divergent in-store head returns
`Error::WriteConflict` and leaves the live statistics unchanged, while a
physically different head with the same verified logical report is accepted.

## Verification

All builds used two Cargo jobs and a private temporary target directory that
was deleted after completion.

```text
cargo fmt --all -- --check
  passed

cargo test --lib application::database::tests
  7 passed

cargo check --all-targets --all-features
  passed

cargo clippy --all-targets --all-features -- -D warnings
  passed

cargo test --all-targets --all-features
  103 library tests passed
  150 integration tests passed
  2 power-loss probe tests passed
  5 benchmark entrypoints remained opt-in

git diff --check
  passed
```

The complete executed total was 255 Rust tests. No benchmark workload, cache
reset, reboot, shutdown, sleep, or power operation ran.
