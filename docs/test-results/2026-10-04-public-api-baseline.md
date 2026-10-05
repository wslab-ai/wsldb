# Public API contract baseline — 2026-10-04

## Subject

- WSLDB commit: `25797b0f6b97cb775b791877f45caadcf8896b2c`
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Host: Apple M1 Max, macOS 26.6.2 (25G83), arm64 APFS
- Public contract: `tests/public_api.rs`
- Contract SHA-256:
  `96ba8a8c712695009e754909c1652f43debbae3bc475c5a86db4132de67d26cf`

## Change-control mechanism

`scripts/release-audit.sh` now rejects a change to the external-consumer
compile contract unless its expected SHA-256 is deliberately updated. The same
audit requires `tests/public_api.rs` to be included in the Cargo package. This
makes a contract edit visible during review and ensures the published source
contains the test that defines the promised surface.

The hash is not an API-discovery mechanism: a new crate-root export must be
added to the compile contract during review. It is also not a declaration that
the unreleased API is already stable. The first approved release artifact will
be the semantic-versioning baseline.

## Verification

The following commands passed on the subject commit:

```text
bash -n scripts/release-audit.sh
scripts/release-audit.sh
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo test --test public_api --all-features
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

The public API integration test executed three tests: the complete external
consumer surface compiled, all promised `Send + Sync` contracts held, and the
documented `Clone`, `Copy`, `Debug`, `Default`, equality, ordering, and iterator
trait implementations remained available to an external crate.
