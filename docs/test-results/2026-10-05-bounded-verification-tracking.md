# Bounded verification tracking — 2026-10-05

Source commit under test:
`018e1e45658b9c89158a1a5566c7465924876114`
(`fix: bound verification page tracking`).

## Risk and contract

Full-tree verification must remember every reachable tree and overflow page to
detect shared references and cycles. That set previously used an ordered tree
whose per-node allocations were implicit and could abort the process on memory
exhaustion. Verification also compared the final entry count with metadata only
after it had reconstructed and passed every value to the visitor.

Verification now:

- uses a hash set and reserves each new page slot fallibly before insertion,
  returning the typed `FileTooLarge` error when capacity cannot be obtained;
- preserves rejection of repeated tree and overflow page references;
- rejects an entry beyond the metadata count before reconstructing its value or
  invoking the visitor;
- keeps traversal and logical-hash order deterministic because traversal still
  follows encoded branch order and the visited set is never iterated.

The set still grows with the number of reachable pages because exact
duplicate-reference detection requires remembering page identity. The change
makes that growth explicit and fallible; it does not claim constant-memory
verification.

## Verification

Cargo was bounded to two jobs. Benchmark and opt-in soak bodies were not
enabled. No reboot, shutdown, sleep, power cut, raw-device operation, or cache
purge was performed.

```text
CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::tree_verify::tests \
  --all-features
  2 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test operations --all-features
  10 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test write_buffer_verification --all-features
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test corruption_fuzz --all-features
  2 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  244 tests passed; 0 failed

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

The public API and persisted main-file and WAL encodings are unchanged. This is
resource-exhaustion and corruption hardening, not measured performance
evidence. Linux, physical power-loss, cold-cache, and pinned RocksDB release
gates remain open.
