# Atomic initialization validation — 2026-10-03

## Scope

This check validates first database creation at every instrumented publication
boundary. It addresses the failure mode where a crash could leave a durable
empty final file before its checksummed metadata had been written.

## Subject

- WSLDB commit: `16cf6eaf4e6ad1998cc0b3b171598d2093af2aba`
- Host: Apple M1 Max, macOS 26.6.2 (25G83), arm64 APFS
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Repository state: clean

## Commands and result

```text
cargo fmt --all -- --check
git diff --check
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
```

All commands passed. The all-target, all-feature run executed 131 tests. Seven
initialization tests cover normal create and reopen, eight competing opener
processes, a partial metadata write, injected pre-publication and
post-publication I/O errors, and process termination at five creation
boundaries.

The checked protocol writes and syncs complete metadata in a temporary inode
before linking that inode at the requested database path and syncing the parent
directory. A pre-link failure leaves the final path absent. A post-link failure
returns `PublicationOutcomeUnknown`, and reopening validates the complete empty
database. Existing zero-length final files continue to fail closed.

## Limitations

The crash tests terminate a process and the fault tests inject deterministic
errors. They prove recovery semantics at the instrumented boundaries but do not
simulate loss of power, volatile device caches, or filesystem-specific ordering
under power failure. The APFS, ext4, and XFS power-loss gate therefore remains
open.
