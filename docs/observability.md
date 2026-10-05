# Observability

`Database::operational_stats` captures the latest durable head, performs a full
uncached verification, and returns a consistent storage snapshot. It reports
physical file bytes, the committed boundary, reachable live bytes, retired
committed bytes, interrupted-commit tail bytes, entry count, tree depth, and
tree/overflow page counts.
The verification section also contains a SHA-256 digest over a domain tag and
the length-prefixed ordered key/value stream.

The same report includes shared decoded-page cache capacity, residency, hits,
and misses, plus process-local successful commit and combined sync latency
counters. Buffered mode additionally divides successful commits into exact
`wal_preflights`, `wal_encodes`, `wal_writes`, `wal_syncs`, and
`wal_publications` timings. These cover pre-encoding work plus append-boundary
validation, canonical encoding, the positioned write, `File::sync_all`, and
post-sync publication.
Their operation counts match successful ordinary or grouped WAL appends.
Direct copy-on-write commits leave all five counters at zero. The report also
includes primary-tree checkpoint starts, currently active work, and successful
publication latency. A checkpoint is successful only after its metadata sync
completes; WAL snapshot consolidation is not misclassified as a tree
checkpoint. The same start and in-progress counters, successful histogram,
automatic-failure count, and cumulative checkpoint data-plus-metadata bytes are
available through `Database::checkpoint_stats`. That method reads only bounded
atomic counters and does not scan or verify the database, so monitoring and
benchmark harnesses can sample it between workload windows.

The report also includes logical mutation bytes and the data-page and metadata
bytes emitted by successful commits. Data-page bytes are
split into leaf, branch, and overflow-page bytes; durable write-buffer bytes are
reported separately, including bytes written by explicit buffer consolidation.
Automatic checkpoint failures are counted without converting an
already synced buffered commit into a false failure. Capacity and residency
exclude the bounded 64-page cache held by each reader thread; hits include both
cache levels.
Latency counters expose operation count, total nanoseconds, most recent latency,
maximum latency, and p50/p95/p99/p99.9 estimates. Percentiles are conservative
upper bounds from a fixed lock-free logarithmic histogram with 16 buckets per
power of two. A non-zero estimate is no more than 6.25% above the recorded
value at that rank. Recording adds one relaxed atomic increment per measured
phase, performs no allocation, and retains a bounded memory footprint.

The write counters describe bytes submitted by WSLDB to its database file and
optional write-buffer sidecar.
They do not include filesystem, block-device, or flash-translation-layer write
amplification. Divide `writes.database_bytes()` by
`writes.logical_mutation_bytes` for the process-local database write
amplification when the logical denominator is non-zero.

Read paths and explicit verification increment a process-local corruption
detection counter whenever they return `Error::Corrupt`. I/O failures and input
errors are intentionally not classified as corruption.

Because the call verifies every live page and value, it is an operational audit,
not a cheap hot-path metric scrape. `Database::stats` and
`Database::page_cache_stats` remain inexpensive alternatives;
`Database::checkpoint_stats` is the inexpensive checkpoint-specific view. The
public database-state lock is released after capturing the storage generation,
so the audit does not by itself delay publication into the `Database` handle.
