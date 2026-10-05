# RocksDB WAL timing diagnostic preregistration

Date: 2026-10-04. This document is committed before adapter instrumentation
or measurement.

## Observation

Exact WSLDB timing attributed 94.038% of one clean short W6 commit interval to
`File::sync_all`, 1.877% to the WAL write, and 4.085% to the remaining commit
path. The fixed-extent diagnostic then failed its gate, so file-size metadata
updates are not a demonstrated explanation for the synchronization cost.

The pinned RocksDB source exposes `WAL_FILE_SYNCED`, `WAL_FILE_BYTES`, and the
`WAL_FILE_SYNC_MICROS` histogram. On macOS, both its default `Sync` path and
Rust's file synchronization use `F_FULLFSYNC`. Exact RocksDB timing is needed
before changing WSLDB's durable-write architecture.

## Diagnostic

Add opt-in instrumentation to the existing RocksDB sustained adapter. When
`WSLDB_ROCKSDB_WAL_STATS=1` is set, the adapter will:

- attach RocksDB's own `Statistics` implementation before open;
- retain the normal `WriteOptions::sync = true` path;
- report WAL synchronization count, WAL bytes, and the count, sum, average,
  p50, p95, p99, and maximum of `WAL_FILE_SYNC_MICROS`;
- leave instrumentation disabled by default so the frozen comparison adapter
  retains its previous overhead.

The report is diagnostic output only. It must not alter workload bytes,
operation selection, durability, or database options other than collecting
statistics.

## Interpretation gate

A short alternating W6 run may route subsequent optimization work, but it is
not an acceptance benchmark:

- if RocksDB's mean WAL sync is repeatedly at least 8% lower, investigate its
  concrete file-I/O and WAL lifecycle path before changing WSLDB;
- if the means remain within 8%, prioritize WSLDB preflight/publication work
  and concurrent group commit rather than a different sync primitive;
- if results change sign with order, classify the short run as filesystem
  noise and make no production change.

Every run must preserve one million entries and pass each engine's existing
final checksum, logical-hash, and reopen checks. The adapter must compile with
the pinned clean RocksDB checkout, and `git diff --check` must pass.
