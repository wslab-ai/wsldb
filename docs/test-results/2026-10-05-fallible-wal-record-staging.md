# Fallible WAL-record staging verification — 2026-10-05

Source commit under test:
`6b4f8d6afeab38b69c7bd8b9808a1b146d4ee0b4`
(`fix: reserve buffered wal record staging`).

## Risk and contract

Application-level group staging already reserved its bounded vectors
fallibly, but the buffered storage adapter still grew its `Vec<WalRecord>`
with an ordinary `push`. Incremental refresh did the same while retaining
records decoded from a concurrently appended WAL tail. Both paths could ask
the allocator for a larger transaction-count-sized contiguous buffer without
first obtaining a typed failure.

New group commit now reserves space for every batch before moving any change
set into a WAL record. A reservation failure returns `Error::FileTooLarge`
before WAL encoding or publication. Incremental refresh performs a fallible
one-record capacity reservation inside the replay callback before accepting
each decoded record. If that reservation fails, refresh returns without
mutating the in-memory write-buffer state; the already durable WAL remains the
recovery authority.

The WAL encoding, checksums, transaction ordering, persisted bytes, and public
API are unchanged. This protection covers record-vector capacity. It does not
claim that every allocation used by the logical records or standard-library
ordered maps is recoverable under system-wide out-of-memory conditions.

## Verification

Cargo work was limited to two build jobs. Benchmark, soak, physical power-loss,
shutdown, and reboot modes were not invoked.

```text
CARGO_BUILD_JOBS=2 cargo test --test group_commit --all-features
  12 passed; 0 failed

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

This closes the identified infallible record-vector growth in new group
assembly and incremental WAL refresh. External Linux, filesystem power-loss,
cold-cache, and pinned RocksDB release gates remain open.
