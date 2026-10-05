# Changelog

## Unreleased

- Replace whole-snapshot commits with an incremental copy-on-write B+ tree.
- Add immutable checksummed leaf, branch, and overflow pages.
- Add multi-level splits, ordered traversal, deletion, and root collapse.
- Rebalance sparse leaf and branch pages after deletes through merge or
  byte-balanced redistribution.
- Support values up to 64 MiB through checksummed overflow chains.
- Serialize writers across processes with standard-library file locks.
- Detect stale writers opened through independent database handles.
- Add full-tree verification with structural and overflow-chain checks.
- Add a configurable bounded decoded-page cache with runtime counters.
- Add bounded ordered range scans with branch-level pruning.
- Persist only the final reachable page graph from each batch commit, avoiding
  intermediate copy-on-write page amplification.
- Add a dependency-free W1/W3/W4/W7 benchmark harness with deterministic data,
  warm-up runs, latency percentiles, cache counters, and file-size reporting.
- Add opt-in subprocess crash injection after page writes and persistence
  boundaries, with atomic old-or-new snapshot recovery checks.
- Add non-overwriting online backup with a short root-capture lock, full
  destination verification, sync, and atomic final-path publication.
- Share cached decoded nodes by `Arc` so cache hits do not copy page-sized tree
  nodes; mutable copy-on-write paths still clone explicitly.
- Add bounded-memory, bottom-up snapshot compaction into an atomically published
  standalone file, including overflow values and full destination verification.
- Add multi-process writer stress with retry-on-conflict and full post-run
  verification of every durable commit.
- Add full operational statistics for space amplification, tree shape, page
  cache behavior, commit/sync timing, and corruption detections.
- Add lightweight checkpoint start, in-progress, successful/failed, and
  written-byte counters, then annotate each sustained-workload latency window
  with checkpoint activity across compaction-driven handle replacement.
- Make sustained comparison inputs fail closed against inherited benchmark
  controls, empty or duplicate distributions, and unrecorded RocksDB WAL
  statistics; record the seed and host/filesystem context in every manifest.
- Make RocksDB W5/W6 delete/recreate perform the same measured exact existence
  lookup as WSLDB before its durable delete; earlier sustained ratios remain
  historical diagnostics and cannot satisfy the release matrix.
- Specify every pre-release format field and add golden metadata and leaf byte
  vectors.
- Add opt-in ten-million-operation model soak and arbitrary-byte/truncation
  corruption fuzz harnesses with deterministic seeds.
- Make cache hits concurrent through a read lock and atomic LRU access stamps;
  extend W3 benchmarks to 1, 4, 16, and 64 reader threads with wall-clock rates.
- Add a streaming SHA-256 logical digest to full verification, backup,
  compaction, and operational reports.
- Add a stable cross-process sidecar lock and atomic active-generation
  compaction on Unix; old readers remain valid, stale writers are rejected, and
  crash injection covers pre-publication and rename boundaries.
- Add model-based mixed mutation, multi-level tree, overflow corruption, and
  root recovery and concurrent-handle tests.
- Add an optional synced write buffer with deterministic WAL recovery,
  checksummed consolidation, and copy-on-write checkpoints.
- Validate buffered transactions and affected indexes before their durable
  sync so acknowledged commits cannot turn into post-sync error returns.
- Create invisible ordered write-buffer nodes before WAL synchronization, then
  publish only prepared slots and discard them after a failed append.
- Replay only newly appended WAL records when a stale buffered handle refreshes
  within the same storage generation.
- Retain the warm decoded-page cache across WAL consolidation and page-tree
  checkpoints that do not replace the primary file generation.
- Strengthen multi-process stress with synchronized independent writers, exact
  acknowledged-commit comparison, abort-on-drop checks, and hang timeouts.
- Count the durable WAL in sustained-comparison space totals and add a fixed
  operation mode that requires WSLDB and RocksDB to produce identical logical
  hashes before their timed results can be trusted.
- Make sustained warm-cache measurements preload and count the complete dataset
  through both engines before timing; RocksDB no longer starts warm while WSLDB
  starts cold.
- Stream consolidated WAL snapshots through bounded staging memory and share
  immutable version storage across generation handoff without changing the
  persistent encoding or old-reader visibility.
- Prefix-compress sorted WAL keys against the preceding mutation and retain
  strict reconstruction, ordering, length, and checksum validation.
- Store grouped transactions in one atomically recovered WAL frame with shared
  checksums, canonical varints, and cross-transaction key-prefix compression.
- Let buffered active compaction catch up durable commits made during its
  snapshot rebuild, report the publication-lock pause, and retain direct-mode
  stale-publication rejection.

## Initial prototype

- Add a functional embedded byte-key/byte-value database API.
- Add immutable read snapshots and a single process-local writer.
- Add durable copy-on-write snapshot commits with two checksummed metadata
  slots and recovery fallback.
- Add ordered iteration, commit, abort, delete, reopen, and corruption tests.
- Document the development architecture and limitations.
