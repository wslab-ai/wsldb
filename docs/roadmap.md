# Roadmap

## Durable transactional core

- [x] Ordered byte key/value API
- [x] Immutable read transactions
- [x] Single process-local writer
- [x] Commit and abort
- [x] Alternating metadata slots
- [x] CRC32C validation and recovery fallback
- [x] Reopen and corruption tests

## Page engine

- [x] Checksummed branch and leaf pages
- [x] Copy-on-write B+ tree insert, split, delete, and root collapse
- [x] Overflow pages for large values
- [x] Cross-process writer locking and stale-writer rejection
- [x] Full-tree structural and value verification
- [x] Bounded ordered range scans with subtree pruning
- [x] Delete merge/redistribution and occupancy recovery
- [x] Reader-safe append-only pages with atomic generation compaction
- [ ] Optional page reuse with a cross-process oldest-reader protocol
- [x] Atomic active-generation compaction with stale-writer rejection on Unix
- [x] Configurable bounded decoded-page cache
- [ ] mmap adapter behind an audited safety boundary
- [x] Differential mutation tests against an independent ordered model
- [x] Process-crash injection at page writes and persistence boundaries
- [x] Direct/WAL short-write, disk-full, and commit-sync failure injection
- [x] Checkpoint, backup, compaction, replacement, and directory-sync failures
- [x] Multi-process writer serialization stress
- [x] Storage amplification and runtime observability snapshot
- [x] Written pre-release format specification and encoder golden vectors
- [x] Opt-in model-soak and corruption-fuzz harnesses
- [ ] Power-loss testing on the supported filesystem matrix
- [x] Deterministic baseline benchmark harness for W1/W3/W4 and clean W7

## Advanced storage capabilities

- [x] Verified snapshot-consistent full backup
- [x] Verified snapshot compaction to a new file
- [ ] Configurable page sizes up to 64 KiB
- [x] Page-level checksums bound to physical page IDs
- [ ] Encryption provider boundary and authenticated encryption
- [ ] Incremental backup based on page transaction IDs
- [ ] Nested read transactions under a writer
- [ ] Two-phase commit and recovery
- [ ] Cross-process reader table
- [x] Durable group commit with explicit completion and error semantics
- [x] Automatic foreground coalescing for concurrent blind-write batches

Performance gates will be defined before benchmark results are collected. No
threshold will be tuned after observing held-out benchmark workloads.
