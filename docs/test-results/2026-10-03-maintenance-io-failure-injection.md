# Maintenance I/O failure injection — 2026-10-03

## Subject

- WSLDB commit: `16cf6eaf4e6ad1998cc0b3b171598d2093af2aba`
- Host: Apple M1 Max, macOS 26.6.2 (25G83), APFS
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Repository state: clean

## Fault matrix

The opt-in subprocess matrix injected one `StorageFull` error at each of these
boundaries:

- partial backup metadata and data writes;
- partial compaction data-page and metadata writes;
- compaction data sync, metadata write and sync, final truncate and sync;
- snapshot hard-link, both directory syncs, and temporary-link removal;
- backup destination sync;
- active-generation rename and its directory sync;
- buffered checkpoint data sync, metadata write and sync, and WAL clear;
- WAL snapshot sync, rename, and directory sync.

Every source and every target that could have been published was reopened and
fully verified. Failures before link or rename returned `Error::Io` and did not
publish a target. Failures after publication returned
`Error::PublicationOutcomeUnknown`, and the complete verified target or active
generation was present.

## Result

All eight focused integration tests passed. The complete all-target,
all-feature suite passed 131 tests, followed by Clippy with warnings denied,
`cargo check`, and rustdoc with warnings denied. Formatting and diff checks also
passed.

The matrix found one live-handle defect: a checkpoint metadata-sync error could
leave the main file advanced while the in-memory store retained the earlier
checkpoint. The commit was durable but later operational inspection returned a
write conflict. The failure path now reconstructs the buffered store from the
verified main file and WAL while still holding the writer and process lock.
The triggering commit stays successful, `checkpoint_failures` increments, and
the same handle remains readable and writable.

This is deterministic failure simulation, not evidence for a physically full
filesystem, device failure, kernel crash, or power removal.
