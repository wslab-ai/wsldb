# Windows cross-compilation check — 2026-10-03

## Scope

This check compiles every library, test, and benchmark target through the
Windows-specific Rust branches. It is a compile-time portability gate only: it
does not execute the test binaries, exercise NTFS, or validate Windows crash
and power-loss behavior.

## Subject

- WSLDB commit: `df81a6927d5a4b2c5009a40beda7999d7dcec9bd`
- Host: Apple M1 Max, macOS 26.6.2, arm64 APFS
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Target: `x86_64-pc-windows-gnu`
- Repository state: clean

## Commands and result

```text
RUSTFLAGS='-D warnings' \
  cargo check --all-targets --all-features \
  --target x86_64-pc-windows-gnu

RUSTFLAGS='-D warnings' \
  cargo clippy --all-targets --all-features \
  --target x86_64-pc-windows-gnu -- -D warnings
```

Both commands passed. The check covers the non-Unix active-file replacement,
directory-sync, WAL-generation, explicit group-commit, test-fixture, and
file-lock compilation paths. Unix-only imports are conditionally compiled, so
the target is warning-free with warnings denied.

The same source passed the complete 122-test all-target/all-feature suite on
the macOS host before the clean cross-check. Windows execution, finite model
and corruption gates, multi-process stress, and NTFS power-loss testing remain
open and must not be inferred from this result.
