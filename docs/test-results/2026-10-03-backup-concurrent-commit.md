# Backup during a concurrent commit — 2026-10-03

## Scope

This check proves that online backup retains one immutable logical snapshot
while another process commits to the source. It covers direct page copying and
the buffered path that materializes checkpoint plus WAL entries through the
bulk loader.

## Subject

- WSLDB commit: `322a93601d3447e73b2dce450ed9d32e27b5907b`
- Host: Apple M1 Max, macOS 26.6.2 (25G83), arm64 APFS
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Repository state: clean

## Method and result

A backup subprocess captured the source snapshot and then paused after its
first destination write. For a direct database this occurred inside immutable
page copying. For a buffered database it occurred after the logical snapshot
had been collected and while compact pages were being emitted.

While the backup worker remained paused, the parent process committed a new key
and observed its new transaction ID. After backup resumed, the source contained
the new transaction and one additional entry. The standalone backup retained
the earlier transaction ID and entry count, matched the earlier complete
logical SHA-256, and did not contain the concurrent key. No destination
temporary file remained.

```text
cargo fmt --all -- --check
git diff --check
cargo check --all-targets --all-features
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
```

All commands passed. The all-target, all-feature run executed 143 tests,
including the worker and both deterministic backup race cases.

This is interprocess snapshot-isolation evidence on APFS. It is not a
power-loss or performance result.
