# Release-profile test suite — 2026-10-03

## Scope

This check executes every ordinary unit, integration, crash-injection,
fault-injection, and benchmark harness target using the optimized release
profile. It checks that the behavior validated in development builds is
preserved under release optimization and all feature combinations.

## Subject

- WSLDB commit: `33cd8ad21ef44b2e5e2abafe05111da89025d8a5`
- Host: Apple M1 Max, macOS 26.6.2 (25G83), arm64 APFS
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Repository state: clean

## Command and result

```text
cargo test --release --all-targets --all-features
```

The command passed. It compiled the release profile with thin LTO and one
code-generation unit, then executed 135 tests with no failures. This includes
process-crash, short-write, sync-failure, corruption, multi-process writer,
backup, compaction, write-buffer, group-commit, refresh, fixture-decoder, and
page-tree coverage. Both benchmark harnesses also started successfully and
exited through their explicit opt-in guards.

## Limitations

The long finite model and corruption workloads are environment-variable gated.
Their test functions return successfully without executing the large workload
when those variables are absent, as they were here. Their separately recorded
10-million-operation and 200,000-mutation evidence remains authoritative; this
release-profile suite does not replace it. It is also not power-loss or
comparative performance evidence.
