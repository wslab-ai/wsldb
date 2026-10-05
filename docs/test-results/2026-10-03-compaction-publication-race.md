# Compaction publication race — 2026-10-03

## Scope

This check proves that a compacted generation built from an older snapshot
cannot replace a commit published while compaction is running. It covers both
the direct page-tree path and a database with outstanding durable WAL state.

## Subject

- WSLDB commit: `ec46a8f766c5757d1705740f035c762c75a76a49`
- Host: Apple M1 Max, macOS 26.6.2 (25G83), arm64 APFS
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Repository state: clean

## Method and result

A subprocess captured the source state, built and fully verified a compacted
replacement, then paused at the existing `compaction_ready` boundary without
holding the database writer lock. The parent process committed a new key and
allowed the compactor to continue.

In both direct and buffered cases, the compactor reacquired the stable sidecar
lock, detected the changed metadata head or WAL length, returned
`Error::WriteConflict`, and removed its replacement. Reopening preserved the
concurrent transaction ID, entry count, and value. No replacement or nested
temporary file remained.

```text
cargo fmt --all -- --check
git diff --check
cargo check --all-targets --all-features
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
```

All commands passed. The all-target, all-feature run executed 140 tests,
including the worker and both deterministic race cases.

This is an interprocess publication-ordering check on APFS. It is not a stress,
performance, or power-loss result.
