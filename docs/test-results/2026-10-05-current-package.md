# Current package artifact verification

Date: 2026-10-05

## Source

- commit: `028ad78a15609443cc267e43732282dba68f8dc5`;
- worktree before packaging: clean;
- host: `Darwin 25.6.0 arm64`;
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`;
- Cargo: `cargo 1.89.0 (c24e10642 2025-06-23)`.

## Artifact

`cargo package` created `wsldb-1.0.0.crate` from the clean source and then
successfully compiled the unpacked archive:

```text
Packaged 328 files, 1.8MiB (420.4KiB compressed)
Verifying wsldb v1.0.0
Finished `dev` profile [unoptimized + debuginfo]
```

- archive size: 430,513 bytes;
- SHA-256:
  `0ff906e638f14f1fce16d21e096fa4ee730cc1f841628157bac5b31624c80dc6`.

The archive contains `LICENSE`, `README.md`, the file-format and API-stability
documents, all three committed database/WAL fixture files, and the external
consumer compile contract. It contains no build directory, repository
metadata directory, CI configuration, or notice file.

## Scope

The command used a private temporary target with two build jobs. The target and
archive were removed after inspection. No benchmark, release-profile test,
cache reset, shutdown, reboot, or power operation was run.

This proves the current macOS package construction and unpacked-source compile
steps only. It does not close the release-profile, Linux, real power-loss,
cold-cache, or pinned RocksDB gates.
