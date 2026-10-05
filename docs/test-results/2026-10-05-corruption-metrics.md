# Structural corruption metrics

Date: 2026-10-05

## Risk and change

`OperationalStats::corruption_detections` promised corruption errors observed by
reads and explicit verification. The observation boundary counted
`Error::Corrupt` but omitted `Error::TreeDepthExceeded`, even though the latter
is the fail-closed result for a structurally corrupt tree that exceeds the
bounded traversal depth.

The file storage adapter now uses one corruption classifier for verified-open
fallback and runtime observation. Both `Error::Corrupt` and
`Error::TreeDepthExceeded` increment the process-local counter; unrelated input
errors do not. A focused unit test preserves that classification.

## Verification

All Cargo commands used at most two jobs and private temporary target
directories that were deleted after completion.

```text
cargo test --all-targets --all-features
  106 library tests passed
  152 integration tests passed
  2 power-loss probe tests passed
  5 benchmark entrypoints remained opt-in

cargo check --all-targets --all-features
  passed

cargo clippy --all-targets --all-features -- -D warnings
  reported one redundant closure

mechanical closure removal, then:

cargo test --all-features --lib \
  structural_corruption_variants_increment_the_counter
  1 passed

cargo check --all-targets --all-features
  passed

cargo clippy --all-targets --all-features -- -D warnings
  passed

cargo fmt --all -- --check
  passed

git diff --check
  passed
```

The full executed suite contained 260 Rust tests. No benchmark workload, cache
reset, reboot, shutdown, sleep, or power operation ran.
