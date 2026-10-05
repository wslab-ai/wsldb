# Export metadata-slot parity

Date: 2026-10-05

## Risk

Normal commits publish transaction `N` in metadata slot `N % 2`, preserving the
other slot as the previous recovery head. Initialization happened to follow the
same rule for transaction zero, but raw backup and both compaction builders
always wrote their sole head to slot zero.

An export made at an odd transaction ID therefore placed transaction one in
slot zero. Its next transaction was even and also targeted slot zero. A partial
metadata write could invalidate that only head, leaving neither the old nor the
new snapshot openable despite all data pages having been written copy-on-write.

## Change

`metadata::slot_offset` is now the single implementation of metadata-slot
selection. Initialization, direct commit, checkpoint publication, raw backup,
direct compaction, and layered buffered compaction all use it. A single-head
file writes its head to `transaction_id % 2` and leaves the opposite slot
invalid, so its first later commit necessarily targets the other slot. Opening
also rejects a checksummed head found in the wrong slot; malformed input cannot
reintroduce the single-slot overwrite hazard.

## Causal coverage

The regression creates transaction-one outputs through raw backup, direct
compaction, and buffered layered compaction. For each output, subprocess fault
injection stops the next direct commit after either one or 4,095 metadata bytes
have been written. Recovery must open and fully validate either transaction one
or transaction two. A unit test separately pins the zero/one/two slot mapping.
Another integration test moves an otherwise valid transaction-one head to slot
zero, clears slot one, and requires open to return `Error::Corrupt`.

## Verification

All Cargo commands used at most two jobs and private temporary target
directories that were deleted after completion.

```text
cargo test --all-features --test io_failures \
  partial_metadata_writes_preserve_odd_export_heads
  1 passed

cargo test --all-features \
  infrastructure::metadata::tests::transaction_parity_selects_the_metadata_slot
  1 passed

cargo test --all-features --test metadata_slots
  1 passed

cargo test --all-targets --all-features
  107 library tests passed
  155 integration tests passed
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

The complete executed suite contained 264 Rust tests. No benchmark workload,
cache reset, reboot, shutdown, sleep, raw-device, or power operation ran.
