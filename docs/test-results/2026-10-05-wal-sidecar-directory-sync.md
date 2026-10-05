# Empty WAL sidecar directory durability

Date: 2026-10-05

## Risk

The first buffered open creates `<database>.wal` and must make that directory
entry durable before a WAL commit can be acknowledged. Previously, a parent-
directory sync failure returned an error but left the empty sidecar in place.
The next open saw that the path existed and skipped the directory barrier. It
could then append and sync a commit even though POSIX durability did not yet
guarantee that the sidecar name would survive a power loss.

## Change

WAL open now treats both a missing sidecar and an existing empty sidecar as
requiring parent-directory synchronization. An injected failure leaves the
file empty. Every later open repeats the barrier until it succeeds, and no WAL
record is appended before that success. A nonempty WAL does not repeat the
barrier because reaching a prior append already required a successful empty-
sidecar open.

## Causal coverage

The regression initializes the main database, then injects the directory-sync
failure in two independent subprocess opens. Both opens must fail with the
configured I/O error and leave a zero-length WAL. A normal buffered open then
commits one value; default recovery must return that value and full
verification must pass. The fault-boundary ownership contract also names the
new injection stage, so future unowned durability boundaries fail the complete
suite.

## Verification

All Cargo commands used at most two jobs and private temporary target
directories that were deleted after completion.

```text
cargo test --all-features --test io_failures \
  empty_wal_retries_a_failed_directory_sync_before_commit
  1 passed

cargo test --all-features --test injection_contract \
  every_fault_boundary_has_an_explicit_test_owner
  1 passed

cargo test --all-targets --all-features
  107 library tests passed
  156 integration tests passed
  2 power-loss probe tests passed
  5 benchmark entrypoints remained opt-in

cargo check --all-targets --all-features
  passed

cargo clippy --all-targets --all-features -- -D warnings
  passed

RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
  passed

cargo fmt --all -- --check
  passed

git diff --check
  passed

scripts/release-audit.sh
  passed
```

The complete executed suite contained 265 Rust tests. No benchmark workload,
cache reset, reboot, shutdown, sleep, raw-device, or power operation ran.
