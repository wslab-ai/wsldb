# Commit-path I/O failure injection — 2026-10-03

## Subject

- WSLDB commit: `f7d264859765ee15dcf583506bc07dc689d0d725`
- Host: Apple M1 Max, macOS 26.6.2 (25G83), APFS
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`

## Fault model

The opt-in `fault-injection` feature imposed one `StorageFull` error at a
specified positioned-write byte offset or commit sync boundary. Every case ran
in a subprocess so counters and environment were isolated. The normal build
does not inspect fault-injection environment variables.

The direct path failed data or metadata writes after 1, 4,095, 4,097, and 8,191
bytes. The buffered path failed WAL writes after 1, 51, 67, and 512 bytes.
Additional cases failed direct data sync, direct metadata sync, and buffered WAL
sync. Reopened state was fully verified after every case.

## Result

- Partial data-page and WAL writes returned `Error::Io` and preserved the
  previous committed snapshot.
- A partial metadata-page write exposed that all meaningful metadata bytes can
  reach storage before failure on trailing padding. It now returns
  `Error::CommitOutcomeUnknown` rather than falsely promising rejection.
- Direct metadata-sync and buffered WAL-sync errors return the same explicit
  ambiguous-outcome error. Refresh selected the complete new transaction in
  these graceful-process cases.
- Incomplete WAL suffixes were rolled back, and both direct and buffered live
  handles successfully committed a later transaction without reopening.
- Six focused integration tests passed. The complete all-feature suite passed
  103 tests, followed by Clippy with warnings denied and `cargo check`.

No torn case produced a mixed logical snapshot or unverified bytes. This closes
commit-path short-write and sync-error simulation. Maintenance-path coverage is
recorded separately. Neither simulation represents a real full filesystem,
device failure, kernel crash, or power loss.
