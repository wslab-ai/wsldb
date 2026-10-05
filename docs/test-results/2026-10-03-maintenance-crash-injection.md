# Maintenance process-crash injection — 2026-10-03

## Scope

This check validates backup and compact-output behavior when the process exits
at every instrumented write and publication boundary. It complements injected
I/O errors: the worker is aborted without unwinding, so temporary-file cleanup
cannot be responsible for the observed final-path state.

## Subject

- WSLDB commit: `470295f8f7ac46c700bf94bba00619991b563893`
- Host: Apple M1 Max, macOS 26.6.2 (25G83), arm64 APFS
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Repository state: clean

## Matrix and result

The subprocess matrix enumerated every backup-copy write and every compacted
page write until a worker completed normally. It also aborted after backup
sync, compaction data sync, metadata write, metadata sync, truncate, final sync,
snapshot hard-link, both parent-directory syncs, and temporary-name removal.

Before the hard-link boundary, every crashed run left the final target absent.
At and after hard-link publication, the target reopened as a complete database
with the same fully verified logical SHA-256 as the source. The source hash was
unchanged by all runs.

```text
cargo fmt --all -- --check
git diff --check
cargo check --all-targets --all-features
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
```

All commands passed. The complete all-target, all-feature run executed 135
tests, including four maintenance-crash tests.

## Limitations

This proves process-crash behavior at the instrumented boundaries on APFS. It
does not model device-cache loss, torn sectors, filesystem bugs, or actual power
removal. The filesystem-specific power-loss gate remains open.
