# Bounded range and cursor staging — 2026-10-05

Source commit under test:
`b5114c72c523754fcde51185ff11b5c0b47e1a24`
(`fix: bound range and cursor staging`).

## Risk and contract

The base-tree range cursor initially reserved at most 1,024 result slots, but
later `Vec::push` growth up to a caller-provided limit used infallible implicit
reallocation. A full scan reserved the metadata `entry_count` exactly, yet a
checksum-valid corrupt tree containing more entries could exceed that capacity
and continue reading more pages before the final count mismatch was detected.
The streaming visitor had the same late count check and could continue
compaction work beyond the declared snapshot size.

Buffered range merge then converted the already sorted base range into a
`BTreeMap`, applied sorted buffered versions, and collected a second result.
The tree's node allocations and the final collection could not be reserved
fallibly. This also duplicated ownership staging for a result that is naturally
a two-way merge.

The cursor now:

- reserves every range-result growth with `try_reserve`;
- rejects a full scan as soon as it would exceed metadata `entry_count`;
- stops a streaming visit at the same boundary before invoking the visitor or
  following another page.

Buffered range merge now walks the owned base iterator and visible version-map
range in key order. Insertions, replacements, tombstones, historical snapshot
visibility, end bounds, and result limits are resolved while producing one
fallibly growing output vector. Buffered key and value copies use the existing
fallible byte-clone helper. No `BTreeMap` or second result collection is built.

## Verification

Cargo was bounded to two jobs. Benchmark and opt-in soak bodies were not
enabled. No reboot, shutdown, sleep, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --lib cursor::tests --all-features
  3 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::write_buffer::tests \
  --all-features
  8 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test database --all-features
  21 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test write_buffer --all-features
  16 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  242 tests passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" CARGO_BUILD_JOBS=2 \
  cargo doc --no-deps --all-features
  passed

CARGO_BUILD_JOBS=2 scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

The public API and persisted main-file and WAL encodings are unchanged. The
change closes allocation and corrupt-count handling gaps; no performance claim
is made without a preregistered measurement. Linux, physical power-loss, cold
cache, and pinned RocksDB release gates remain open.
