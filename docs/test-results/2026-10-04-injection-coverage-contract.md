# Injection boundary coverage contract — 2026-10-04

## Subject

- Commit: `6b3de67ad55ae1a28674c41994729254248ac0c3`.
- Host: Apple M1 Max, macOS 26.6.2, APFS.
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`.

## Contract

`tests/injection_contract.rs` recursively scans every production Rust source
file. It extracts calls to `crash_injection::hit`,
`fault_injection::before`, and the literal custom fault selectors used for
partial positioned writes. Every regular production injection call must use a
static lowercase canonical name.

The extracted set must exactly equal the reviewed coverage tables in the test:

- 37 unique process-crash stages;
- 30 unique I/O-failure stages, including `positioned_write`.

Each table row names an integration-test file. The contract requires that file
to contain the exact stage and configure `WSLDB_FAILPOINT` or
`WSLDB_IO_FAILPOINT` as appropriate. A production stage added without a test,
a stale coverage row, a duplicate row, a renamed stage, or a dynamically
constructed regular stage fails the all-target suite.

This is a structural guard. The existing subprocess tests remain responsible
for reaching their assigned hooks, detecting the injected termination or I/O
error, reopening the files, and checking the old-or-new recovery invariant.

## Verification

The following checks passed with at most two Cargo build jobs:

- `cargo fmt --all -- --check`;
- `cargo check --all-targets --all-features`;
- `cargo clippy --all-targets --all-features -- -D warnings`;
- `cargo test --test injection_contract`: 2 tests;
- `cargo test --all-targets --all-features`: 207 tests, including the complete
  crash, fault, corruption, multi-process, format, and power-loss-probe suite;
- `scripts/release-audit.sh`, which also requires the contract source in the
  packaged crate;
- `git diff --check`.

No database bytes, public API, commit path, or injection runtime behavior
changed. This contract does not emulate real power loss or prove device-cache
semantics; the APFS/ext4/XFS physical power-loss gate remains open.
