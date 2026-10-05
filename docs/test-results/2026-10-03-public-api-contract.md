# Public API compile contract — 2026-10-03

## Subject

- WSLDB commit: `b85a1cc06a90802f96b7280b15bc94d63691a329`
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Host: Apple M1 Max, macOS 26.6.2 (25G83), arm64 APFS
- Repository state: clean

## Contract

`tests/public_api.rs` is a separate integration crate and therefore sees WSLDB
as an external consumer does. It type-checks every crate-root export, public
method, report field, error variant, iterator contract, builder, and maintenance
operation. It also asserts `Send + Sync` for every public type intended to move
between or be shared by threads.

Extensible error, verification, backup, compaction, database, cache, latency,
write, and operational reports are `#[non_exhaustive]`. This prevents an
accidental promise that would make adding a diagnostic or metric a breaking
change after the first release.

## Result

```text
cargo fmt --all -- --check
git diff --check
cargo check --all-targets --all-features
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
```

All commands passed. The all-target, all-feature run executed 137 tests,
including the two external public-API contract tests.

This verifies the current source surface and auto-trait promises. It is not a
comparison against an already released crate because no WSLDB release exists
yet; the first published artifact will become the semantic-versioning baseline.
