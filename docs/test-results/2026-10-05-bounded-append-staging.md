# Bounded append-tree staging — 2026-10-05

Source commit under test:
`c8b2a5eb76d30683dc7e5b51efe5b9209553b375`
(`fix: bound append tree staging`).

## Risk and contract

The monotonic-key append fast path may stage a transaction-sized right-edge
rewrite. Its logical leaf vector previously grew with plain `push` after
overflow-value pages could already have been written. Leaf replacement output,
right-spine replacement expansion, new-level child/separator collection, and
branch output also relied on implicit `Vec` growth while page construction was
in progress. Allocation failure could therefore abort the process instead of
returning the storage API's typed `FileTooLarge` error.

The append path now:

- reserves the complete mutation count before storing any appended value;
- reserves each bounded leaf slice before cloning it and each leaf output slot
  before the corresponding page write;
- reserves both right-spine child and separator expansion before consuming
  replacement nodes;
- builds new-level page IDs and separators in pre-reserved vectors while
  moving, rather than cloning, child minima;
- reserves each branch output slot before writing that branch page.

A later allocation failure can leave only unreachable copy-on-write tail pages.
It cannot publish metadata, replace the committed root, or make a partial tree
visible. The next commit continues after the physical tail, and compaction can
reclaim unreachable pages.

## Verification

Cargo was bounded to two jobs. Benchmark and opt-in soak bodies were not
enabled. No reboot, shutdown, sleep, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --test append --all-features
  3 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test database --all-features
  21 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::tree::tests --all-features
  3 passed; 0 failed

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

The public API, tree algorithm, and persisted file and WAL encodings are
unchanged. This is allocation-failure hardening, not measured performance
evidence. Linux, physical power-loss, cold-cache, and pinned RocksDB release
gates remain open.
