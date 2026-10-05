# File-format envelope contract

Date: 2026-10-05

## Audit finding

The format specification stated that the first retained WAL record must always
use the selected page-tree checkpoint as its base. The recovery implementation
correctly also accepts a continuous older prefix left after checkpoint metadata
became durable but before WAL truncation. The old wording contradicted that
crash-safe two-file transition and its existing focused recovery test.

## Contract correction

The specification now distinguishes physical retention from logical replay.
Records remain one continuous chain, but recovery may skip a prefix already
covered by the checkpoint. It requires either an exact checkpoint boundary
with the same entry count or a first newer record based exactly on the
checkpoint before applying a tail. Entirely covered WAL residue is cleared.

No encoded byte or decoder behavior changed.

## Independent fixture oracle

The committed-fixture suite now parses the outer envelopes without using the
production metadata or WAL decoder. It pins:

- the metadata magic, reserved bytes, page size, transaction, root, last-page,
  and entry-count offsets;
- transaction two in slot zero and transaction one in slot one for the direct
  fixture;
- the empty transaction-zero buffered checkpoint in slot zero;
- the exact buffered WAL chain `snapshot(0→2)`, transaction `2→3`, and
  transaction `3→4`;
- each WAL header length, item count, footer magic, repeated length, and the
  absence of trailing physical records.

The existing fixture-open test independently requires logical contents and a
full structural verification through the production decoder.

## Verification

All Cargo commands used at most two jobs and private temporary target
directories that were deleted after completion.

```text
cargo test --all-features --test format_fixtures
  2 passed

cargo test --all-features \
  infrastructure::file_store::buffered::recovery::tests::\
opening_a_wal_with_a_checkpoint_prefix_replays_its_tail
  1 passed

cargo test --all-targets --all-features
  108 library tests passed
  157 integration tests passed
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

The complete executed suite contained 267 Rust tests. No benchmark workload,
cache reset, reboot, shutdown, sleep, raw-device, or power operation ran.
