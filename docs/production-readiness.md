# Production readiness gates

WSLDB must not use the label `stable` or `production-ready` until every required
gate below has reproducible evidence. Passing unit tests alone is insufficient.

## Correctness and durability

- Model-based insert/update/delete, point-read, bounded/open-ended range, and
  final ordered comparisons pass at 10 million operations on the current
  direct-storage code on APFS
  ([current evidence](test-results/2026-10-04-current-apfs-soak.md)) and on an
  earlier build on virtualized Linux/ext4
  ([evidence](test-results/2026-10-03-linux-ext4-validation.md)). The ordinary
  release suite additionally drives direct and durable-buffered engines through
  one deterministic model scenario covering read-your-writes, historical
  snapshots, random range boundaries, checkpointing, full verification, and
  reopen recovery
  ([evidence](test-results/2026-10-05-cross-engine-model-properties.md)).
- The ten-million-operation model workload and 100,000 main-file plus 100,000
  WAL corruption mutations complete on every supported platform without a
  panic, hang, undefined behavior, or silent error. The full finite mutation
  count for the current prefix-encoded WAL passes on APFS
  ([current evidence](test-results/2026-10-04-current-corruption-fuzz.md)); the
  earlier format also passed on virtualized
  Linux/ext4 ([evidence](test-results/2026-10-03-linux-ext4-validation.md)).
- Crash injection covers every page write, data sync, metadata write, and
  metadata sync boundary. Recovery returns either the previous committed state
  or the complete new state, never a mixture. Backup and compact-output
  publication coverage is recorded
  [here](test-results/2026-10-03-maintenance-crash-injection.md). The complete
  WAL-write-before-sync boundary now also proves old-or-new recovery for one
  transaction and no-or-all recovery for a transaction group
  ([evidence](test-results/2026-10-04-wal-pre-sync-crash.md)). WAL clearing is
  also interrupted after the length change but before its sync in checkpoint
  and active-compaction recovery tests
  ([evidence](test-results/2026-10-04-wal-truncate-crash.md)).
  Initial creation likewise exercises termination after its metadata write and
  before the first file sync
  ([evidence](test-results/2026-10-04-initial-metadata-write-crash.md)).
  The ordered first-tree checkpoint fast path additionally exercises both a
  process abort after its streaming positioned write and an injected I/O error
  before that write; recovery retains the complete acknowledged WAL state
  ([evidence](test-results/2026-10-05-streaming-checkpoint-write-injection.md)).
  Streamed WAL consolidation now exercises termination after a staged
  replacement-record write and before its first sync
  ([evidence](test-results/2026-10-04-wal-snapshot-write-crash.md)). A source-
  scanning contract now fails the all-target suite whenever a production crash
  or injected-I/O stage lacks an explicit integration-test owner
  ([evidence](test-results/2026-10-04-injection-coverage-contract.md)).
- Power-loss tests pass on ext4, XFS, and APFS with documented filesystem
  and hardware assumptions. A resumable independent-oracle probe and protocol
  now exist and pass deterministic direct/buffered smoke validation
  ([protocol](power-loss-testing.md),
  [smoke evidence](test-results/2026-10-04-power-loss-probe-smoke.md)). The
  runner now also pins its own identity and seals only complete, contiguous,
  technically verified cycle sets with a final SHA-256 manifest while keeping
  the manual release decision open. Release, benchmark, and power-loss runners
  self-verify every digest and exact evidence file-set membership before
  accepting a seal; missing, extra, duplicate, unsafe, linked, or non-regular
  entries fail closed
  ([sealing evidence](test-results/2026-10-05-power-loss-evidence-sealing.md),
  [integrity evidence](test-results/2026-10-05-evidence-seal-verification.md));
  no real power-removal result has been recorded yet, so this gate remains
  open.
- Injected short writes, `ENOSPC`, generic I/O failures, sync failures, and
  replacement failures preserve a verified committed state. The API documents
  whether a sync error makes commit outcome ambiguous, and a live handle can be
  recovered or explicitly requires reopen. Direct-commit and buffered-WAL
  coverage is recorded [here](test-results/2026-10-03-io-failure-injection.md);
  maintenance and replacement coverage is recorded
  [here](test-results/2026-10-03-maintenance-io-failure-injection.md). Atomic
  first-creation publication and its process-crash limitations are recorded
  [here](test-results/2026-10-03-atomic-initialization.md). Post-publication
  WAL-clear and truncate-sync failures now report an ambiguous publication and
  preserve a refreshable, verified snapshot
  ([evidence](test-results/2026-10-04-wal-truncate-sync-failure.md)). Direct,
  buffered, and group-commit unknown outcomes also fail closed against a stale
  retry before refresh. Automatic batching now also rejects queued and future
  batches at the coordinator until refresh installs recovered durable state
  ([base evidence](test-results/2026-10-04-unknown-commit-retry.md),
  [automatic evidence](test-results/2026-10-05-automatic-unknown-outcome.md)).
  Single-head backup and direct/layered compaction outputs now preserve the
  transaction-parity metadata slot. Injected partial publication of the first
  later commit therefore retains an old-or-new recoverable head
  and opening rejects otherwise valid metadata stored in the wrong slot
  ([evidence](test-results/2026-10-05-export-metadata-slot.md)).
  A new or previously failed empty WAL sidecar must now pass a parent-directory
  sync before any append. Repeated injected failures prove that path existence
  cannot bypass the durability barrier
  ([evidence](test-results/2026-10-05-wal-sidecar-directory-sync.md)).
- The current all-feature crash, I/O-failure, maintenance, and multi-process
  suite also passes on virtualized Linux/ext4
  ([evidence](test-results/2026-10-03-linux-ext4-validation.md)).
- Opening with buffering disabled temporarily replays an existing WAL without
  silently enabling buffered or group-commit semantics. Flush materializes the
  recovered state and returns subsequent commits to direct copy-on-write
  ([evidence](test-results/2026-10-05-direct-mode-wal-recovery.md)).
- Corruption produces a typed error or documented fallback; it never returns
  unverified bytes. Metadata and the selected root are always validated on
  open; `DatabaseOptions::verify_on_open(true)` now additionally verifies the
  complete main+WAL logical snapshot and every overflow value before returning
  a handle. With an initially empty WAL it can expose the newest older fully
  verified metadata snapshot as a non-repairing recovery view; an initially
  nonempty WAL prohibits fallback so newer durable transactions cannot be
  discarded. Recovery views reject mutation but support generation-checked,
  fully verified `backup` and `compact_to` export into a new database. The
  default preserves the measured fast-open path and detects
  deeper corruption on access
  ([evidence](test-results/2026-10-05-verified-open.md)). WAL replay,
  overflow-value reconstruction, group preflight,
  merged scans, and streamed snapshot staging reserve their potentially large
  contiguous buffers fallibly; capacity overflow is returned as a typed error
  before the corresponding bytes are accepted. Fixed-page and WAL decoders
  also prove advertised entry or mutation counts can fit in their remaining
  payload before entering count-controlled work or reserving collections
  ([large-allocation evidence](test-results/2026-10-05-fallible-large-allocations.md),
  [WAL-handoff evidence](test-results/2026-10-05-fallible-wal-handoff-buffer.md),
  [page-count evidence](test-results/2026-10-05-page-decoder-count-bounds.md),
  [WAL-count evidence](test-results/2026-10-05-wal-mutation-count-bounds.md)).
  User values and decoded WAL values can be as large as 64 MiB; public
  mutation, read-your-writes, recovery, point-read, range-merge, and
  maintenance-overlay copies now reserve those contiguous buffers fallibly
  before copying
  ([evidence](test-results/2026-10-05-fallible-value-copies.md)). Automatic and
  explicit group commit now also reserve their bounded queue-drain and
  change-set staging vectors fallibly before entering the storage engine. A
  staging failure reaches every affected waiter without writing the WAL,
  losing a concurrent arrival, or permanently poisoning later groups
  ([evidence](test-results/2026-10-05-fallible-group-staging.md)). The storage
  adapter likewise reserves a complete new group's record vector before
  consuming its change sets. Its cross-batch presence overlay now reserves a
  hash table before WAL work and reuses immutable change-set keys instead of
  allocating duplicate key bytes
  ([overlay evidence](test-results/2026-10-05-fallible-group-overlay.md)).
  Incremental WAL refresh reserves each retained decoded record before
  accepting it
  ([record evidence](test-results/2026-10-05-fallible-wal-record-staging.md)). Durable
  buffer commits now also reserve version/value arenas and point-update staging
  before WAL synchronization; an existing point index reserves new hash slots
  during preflight, and post-sync shard publication no longer allocates 64
  temporary vectors
  ([evidence](test-results/2026-10-05-buffer-publication-preflight.md)). Lazy
  point-index construction now counts and fallibly reserves every shard before
  insertion. A guarded empty slot prevents construction from a stale buffer
  during WAL synchronization, so write-only commits skip point-update staging
  while the first later read builds from the published head
  ([evidence](test-results/2026-10-05-fallible-lazy-point-index.md)). Mutation
  keys are now reference-counted when a change set is formed and the same
  allocation is reused by WAL-overlay ordering, point updates, snapshots, and
  layered compaction; durable publication no longer converts every key from an
  owned vector after synchronization
  ([evidence](test-results/2026-10-05-shared-buffer-keys.md)). Newly observed
  ordered-map keys now receive invisible `None` nodes before WAL encoding and
  synchronization. Every read and maintenance path ignores those nodes until
  durable publication changes the already occupied slot to a version ID;
  failed appends discard them, and live-handle retries after injected partial
  single and group writes are covered
  ([evidence](test-results/2026-10-05-ordered-buffer-node-preflight.md)). A
  materialized point index uses a fixed shard array, and its pre-WAL presence-
  check buffers reserve fallibly before accepting each mutation
  ([evidence](test-results/2026-10-05-fallible-point-index-preflight.md)).
  Direct copy-on-write commit page staging now uses dense virtual-page vectors,
  reserves traversal and remap storage before the first page write, and remaps
  reachable references in constant time without transaction-sized ordered-map
  allocation
  ([evidence](test-results/2026-10-05-bounded-commit-page-staging.md)). The
  application boundary now also rejects a storage adapter that returns a
  transaction ID other than the one exact successor required by a single
  commit or the final successor required by a group. The inconsistent head is
  never installed in the live handle
  ([evidence](test-results/2026-10-05-commit-transaction-contract.md)). The
  bottom-up bulk loader used by initial trees, checkpoints, and compaction now
  reserves leaf, level, branch, and maintenance-chunk staging before the
  corresponding page write can make progress
  ([evidence](test-results/2026-10-05-bounded-bulk-loader-staging.md)).
  The monotonic-key append fast path now reserves transaction-sized leaf and
  right-spine staging before value writes, and reserves each leaf or branch
  output slot before its corresponding copy-on-write page write
  ([evidence](test-results/2026-10-05-bounded-append-staging.md)).
  Base cursor range growth and buffered range-result growth are now fallible;
  buffered ranges use a single sorted merge instead of an allocation-opaque
  ordered map and second collection. Full scans and streaming visits reject
  entries beyond the metadata count before accepting them or following more
  pages
  ([evidence](test-results/2026-10-05-bounded-range-staging.md)). Public callers
  can now traverse a stable snapshot in bounded pages with an exclusive
  continuation key. Direct and layered-buffer paths neither repeat the
  boundary nor read its value
  ([pagination evidence](test-results/2026-10-05-bounded-scan-pagination.md)).
  Full-tree
  verification now tracks reachable pages in a fallibly reserved hash set,
  preserving duplicate-reference detection without ordered-tree node
  allocations. It also rejects entries beyond the metadata count before
  reconstructing or exposing their values
  ([evidence](test-results/2026-10-05-bounded-verification-tracking.md)).

## Concurrency and operations

- Multi-process writer stress proves global serialization and stale-head
  rejection. The current direct and durable-buffer APFS evidence is documented
  [here](test-results/2026-10-03-multiprocess-stress.md); the measured buffered
  refresh improvement is documented
  [here](test-results/2026-10-03-incremental-buffer-refresh.md). Database paths
  are canonicalized before stable sidecars are selected, symbolic-link aliases
  share the same lock and WAL paths, and Unix hard-link aliases fail closed
  after safe cleanup of interrupted-creation temporary links. Unix handles
  also compare an acquired descriptor with the current path before database
  I/O, so accidental sidecar deletion, rename, or replacement requires reopen
  rather than silently creating two writer-lock domains
  ([live-identity evidence](test-results/2026-10-05-lock-sidecar-replacement.md)).
  The main database, lock, and WAL paths must resolve
  to regular files; symbolic-link sidecars and multiply hard-linked sidecars
  fail closed before recovery or publication
  ([path evidence](test-results/2026-10-05-sidecar-file-identity.md),
  [unlink evidence](test-results/2026-10-05-lock-sidecar-unlink.md)). Direct and
  buffered Unix commits, flushes, buffered maintenance, checkpoint
  publication, and static compaction publication additionally compare the
  captured and current main-file inode. An equal-head external replacement
  therefore returns `WriteConflict` before a direct page, buffered WAL,
  checkpoint, or replacement publication. Refresh then opens the replacement
  generation instead of splitting writes and reads across inodes
  ([direct evidence](test-results/2026-10-05-main-file-identity.md),
  [buffered evidence](test-results/2026-10-05-buffered-main-file-identity.md)).
  Buffered Unix commits also compare the captured WAL descriptor with the
  current sidecar before append. A byte-identical WAL replacement makes
  ordinary, explicit-group, and automatic-group commits return
  `WriteConflict` without extending either inode. Operational statistics,
  backup, flush, and both compaction APIs also fail before creating output or
  publishing mixed generations; refresh reopens the current WAL and restores
  writes while rejected keys remain absent
  ([evidence](test-results/2026-10-05-live-wal-identity.md)).
  Refresh now rejects any candidate whose transaction ID is below the live
  handle's current snapshot. When a nonempty WAL disappears while the physical
  checkpoint remains unchanged, the storage adapter returns `WriteConflict`
  without recreating an empty sidecar; restoring the original WAL permits
  refresh and later commits. This prevents an explicit refresh from silently
  rolling a live process back after sidecar removal or rename
  ([evidence](test-results/2026-10-05-monotonic-refresh.md)).
  Equal-ID generation replacement additionally compares fully verified entry
  counts and logical hashes before publication. A valid checksummed WAL with
  the same transaction ID but different contents is rejected; the symmetric
  divergent main-generation case is also covered. Ordinary direct refresh on
  the exact unchanged head takes a no-scan fast path. A different same-ID head
  on the already open store now receives the same full equivalence check as a
  replacement generation; divergent contents are rejected without changing
  the live snapshot, while a verified equivalent maintenance head is accepted.
  External commits remain observable
  ([follow-up](test-results/2026-10-05-monotonic-refresh.md)).
  The in-store same-ID coverage is recorded separately
  [here](test-results/2026-10-05-same-id-refresh-validation.md).
- The page-space strategy prevents reuse of pages visible to readers. Physical
  page IDs are append-only within a generation; verified atomic generation
  compaction reclaims space while old readers retain the replaced inode and
  stale writers fail closed. A free list and cross-process reader table are
  therefore optional future optimizations, not hidden first-release
  requirements
  ([policy audit](test-results/2026-10-05-page-space-policy-audit.md),
  [compaction contract](compaction.md)).
- Backup and restore are snapshot-consistent and verified by full logical
  hashes, including deterministic direct and buffered concurrent-commit races
  ([evidence](test-results/2026-10-03-backup-concurrent-commit.md)). Long
  snapshot operations release the public database-state lock after capturing
  their immutable generation, so same-handle commit publication does not wait
  for an unrelated full read
  ([state-lock evidence](test-results/2026-10-05-snapshot-state-lock.md)).
- Compaction is crash-safe, bounded in temporary space, and preserves readers.
  A deterministic direct and buffered publication race proves that a stale
  replacement cannot overwrite a concurrent commit
  ([evidence](test-results/2026-10-03-compaction-publication-race.md)). Buffered
  maintenance now uses bounded chunks, and active compaction preserves later
  commits through an inode-changing, byte-identical WAL handoff instead of B+
  tree catch-up under the publication lock. Crash and injected-I/O tests cover
  every handoff and main-replacement boundary
  ([evidence](test-results/2026-10-04-wal-handoff-compaction-preregistration.md)).
  Main-file leases also validate the current device/inode after acquiring the
  OS lock, so a waiter opened before a concurrent rename fails closed instead
  of protecting the unlinked old generation
  ([evidence](test-results/2026-10-05-main-file-lease-identity.md)). Occupied
  active-replacement names are skipped, and a construction failure cannot
  delete a pre-existing orphan or unrelated file
  ([evidence](test-results/2026-10-05-compaction-replacement-collision.md)).
  Direct and layered rebuilds now also structurally verify their source stream
  and compare its canonical entry count and logical hash with the fully
  verified destination before publication. This prevents compaction and
  buffered backup from silently normalizing a checksum-valid malformed tree
  ([evidence](test-results/2026-10-05-compaction-source-equivalence.md)).
  The application boundary additionally refuses to install a flush result that
  changes the logical transaction ID or entry count, or an active-compaction
  result whose source ID and verified snapshot report disagree with its input
  and returned head
  ([evidence](test-results/2026-10-05-maintenance-result-contract.md)).
- File growth, live bytes, retired bytes, tree depth, cache hit rate, commit
  latency, sync latency, and corruption counters are observable. Both generic
  corruption failures and bounded-depth structural failures increment the same
  process-local counter
  ([evidence](test-results/2026-10-05-corruption-metrics.md)). Every commit
  and durable-write phase now also exposes bounded p50/p95/p99/p99.9 estimates,
  including the WAL-sync p99 needed to separate storage-barrier tails from
  preflight and publication work
  ([evidence](test-results/2026-10-04-latency-percentiles.md)). Successful
  primary-tree checkpoint starts, active work, successful publications, bytes,
  and automatic failures additionally have a cheap process-local view that
  does not verify the tree. The sustained harness marks every one-second window
  containing checkpoint activity and preserves cumulative counts across active
  compaction. It explicitly drains and reports checkpoint work before replacing
  a handle for compaction and fails if work remains active, allowing mutation
  tails to be correlated without hidden background I/O or full-tree scans in
  the timed interval. The final owner of any buffered storage generation also
  joins its checkpoint worker during destruction, so checkpoint I/O cannot
  continue detached after that owner returns
  ([checkpoint-window evidence](test-results/2026-10-05-checkpoint-window-observability.md),
  [drop-drain evidence](test-results/2026-10-05-checkpoint-drop-drain.md)).

## Compatibility

- The written file-format specification now has committed main-file and WAL
  decoder fixtures in addition to encoder golden vectors. An independent
  fixture-envelope contract pins the documented metadata offsets, alternating
  slot transactions, WAL record headers, footers, lengths, and logical chain
  without calling the production decoder. The first release must explicitly
  freeze them; every later incompatible stable change requires a tested
  migration path before release
  ([current evidence](test-results/2026-10-05-file-format-envelope-contract.md),
  [original evidence](test-results/2026-10-03-format-fixtures.md)).
- The public API follows the documented semantic-versioning policy and has an
  external-consumer compile contract
  ([original evidence](test-results/2026-10-03-public-api-contract.md)). The
  deterministic release audit pins the exact contract source hash and requires
  the contract in the packaged crate, making pre-release API changes explicit
  while leaving the final stability declaration gated on the first approved
  release
  ([baseline evidence](test-results/2026-10-04-public-api-baseline.md),
  [admission-limit evidence](test-results/2026-10-05-public-admission-limits.md),
  [current verified-open addition](test-results/2026-10-05-verified-open.md)).
- The current clean local Rust package and documentation audit and the complete
  195-test release all-feature suite are recorded in the
  [WAL-handoff release audit](test-results/2026-10-04-wal-handoff-release-audit.md).
  A newer local development-head all-feature run contains 275 Rust tests and
  five opt-in benchmark entrypoint checks, including pre-storage transaction-ID
  exhaustion, fail-closed commit
  transaction sequencing, maintenance-result validation, same-ID in-store
  refresh validation, bounded ordered-scan pagination, constant-time automatic
  group queue removal, compaction source
  verification, retry of the empty-WAL
  directory durability barrier, fallible WAL-handoff staging, and shared
  direct/buffered model-property coverage, and deterministic checkpoint-worker
  drain on storage-generation destruction. The same current run also covers
  the streaming first-tree checkpoint write boundary
  ([current evidence](test-results/2026-10-05-transaction-id-exhaustion.md),
  [pagination evidence](test-results/2026-10-05-bounded-scan-pagination.md),
  [group-queue evidence](test-results/2026-10-05-automatic-group-queue.md),
  [streaming-checkpoint evidence](test-results/2026-10-05-streaming-checkpoint-write-injection.md),
  [model-property evidence](test-results/2026-10-05-cross-engine-model-properties.md),
  [admission-limit evidence](test-results/2026-10-05-public-admission-limits.md),
  [format evidence](test-results/2026-10-05-file-format-envelope-contract.md),
  [fallible-handoff evidence](test-results/2026-10-05-fallible-wal-handoff-buffer.md),
  [WAL-directory evidence](test-results/2026-10-05-wal-sidecar-directory-sync.md),
  [verified-open evidence](test-results/2026-10-05-verified-open.md),
  [maintenance evidence](test-results/2026-10-05-maintenance-result-contract.md),
  [refresh evidence](test-results/2026-10-05-same-id-refresh-validation.md),
  [commit evidence](test-results/2026-10-05-commit-transaction-contract.md),
  [compaction evidence](test-results/2026-10-05-compaction-source-equivalence.md)).
  The preceding 245-test run records ordered-node preflight and live-handle
  partial-WAL retry coverage
  ([earlier evidence](test-results/2026-10-05-ordered-buffer-node-preflight.md)).
  The latest WAL pre-sync crash-boundary evidence remains recorded
  [here](test-results/2026-10-04-wal-pre-sync-crash.md). Earlier detailed
  artifact and release-profile evidence remains
  [here](test-results/2026-10-03-release-artifact.md) and
  [here](test-results/2026-10-03-release-profile-suite.md). Clean source commit
  `028ad78a15609443cc267e43732282dba68f8dc5` also produces a 328-file crate
  whose unpacked source compiles
  with Rust 1.89; its exact archive hash and contents are recorded in the
  [current package evidence](test-results/2026-10-05-current-package.md).
- A deterministic local Linux release runner now requires an ext4 scratch
  filesystem, clean commit, bounded build concurrency and free space, retains
  per-step logs plus the packaged crate hash, cleans its private target, and
  seals every successful evidence file in the same final SHA-256 manifest used
  by benchmark runs. A failed suite has no final integrity manifest
  ([integrity evidence](test-results/2026-10-05-benchmark-evidence-integrity.md)).
  It has not yet been executed on the accepted current source, so the current
  Linux repetition gate remains open
  ([runner evidence](test-results/2026-10-04-linux-release-runner.md)).
- Linux and macOS pass the local release checklist. Windows is expected to
  build and run the core database, but it is a best-effort, untested platform
  outside the official support matrix and is not a release gate. The existing
  warning-free cross-compilation result is portability evidence only
  ([evidence](test-results/2026-10-03-windows-cross-check.md)), not a production
  support guarantee. No hosted CI configuration is required by the repository.
  The deterministic, non-CI hygiene and platform commands are documented in
  the [local release checklist](release-checklist.md).

## Performance

The workload definitions in `benchmarks/workloads.md` are frozen before
comparison results are collected. WSLDB and RocksDB must use equivalent
durability; unsynced writes cannot be compared with synced commits.
The comparison runners now fail closed by default unless both source trees are
clean, RocksDB is at the exact official `v11.8.1` commit, and the executed
adapter reports version 11.8.1. Explicit diagnostic mode marks its manifest as
not satisfying the source contract. Even strict individual runs explicitly
record that the complete multi-run release gate remains unproven
([contract evidence](test-results/2026-10-04-pinned-rocksdb-contract.md)). The
sustained runner additionally scrubs inherited benchmark controls, rejects
empty or duplicate workload lists, and records its seed, optional statistics
mode, and host/filesystem context
([sustained-runner evidence](test-results/2026-10-05-sustained-runner-contract.md)).
It now also proves that both setup databases contain the same logical dataset
before timing and rejects missing, duplicated, malformed, or inconsistent
completion records for every W5/W6 process
([result-validation evidence](test-results/2026-10-05-sustained-result-validation.md)).
Each WSLDB process now additionally produces a fail-closed joined
`checkpoint-windows.csv`; missing, duplicated, non-contiguous, or malformed
latency/lifecycle records, including contradictory inactive counters,
invalidate the run before tail-interference analysis
([window-join evidence](test-results/2026-10-05-checkpoint-window-join.md)).
Both comparison runners now hash their orchestration scripts, adapters,
benchmark executables, reset hooks, and common evidence tools. Before sealing,
release-mode runs revalidate those files and both clean source commits; any
mid-run change invalidates the result. They publish a path-sorted SHA-256
manifest covering every final evidence file only after the run completes. An
interrupted or rejected run has no final integrity manifest
([base integrity evidence](test-results/2026-10-05-benchmark-evidence-integrity.md),
[input snapshot evidence](test-results/2026-10-05-runner-input-snapshots.md)).
The short comparison runner can now isolate each cold W3/W4/W7 repetition in a
new process and invokes a hashed, recorded platform reset hook before each
engine. The hook mechanism has deterministic shell coverage; no supported-host
cold-cache result has been recorded yet, so the evidence gate remains open
([protocol evidence](test-results/2026-10-04-cold-cache-protocol.md)).
Fail-closed macOS `purge` and Linux `drop_caches` reference helpers are present,
but are intentionally not executed on a shared or active development host.

Required wins for the primary embedded read profile:

- at least 1.25x random point-read throughput;
- at least 1.25x ordered range-scan throughput;
- lower p99 latency during sustained updates;
- at least 5x faster open-to-first-read time;
- no hidden background work left outside the measurement window.

Write throughput, read amplification, write amplification, space amplification,
RSS, database size, and p50/p95/p99/p99.9 latency must all be reported. A result
that wins throughput by violating durability or using materially more resources
does not pass.

The current exact W7 diagnostic measures the same open-plus-first-point-read
interval in separate processes over byte-identical 1M-entry databases. WSLDB's
206.250-microsecond median was 133.41 times faster than RocksDB 11.8.1's 27.516
milliseconds on the warm-OS-cache APFS host, passing the five-times threshold
for that profile ([evidence](test-results/2026-10-03-current-rocksdb-1m.md)).
Cold-cache and supported-platform coverage remain open.

A separate dependency-free W7 harness now isolates full structural, checksum,
value, and logical-hash verification with an explicit warm-up, repeated opens,
and identity checks between every run. It enables bounded causal verification
measurements without rerunning W5/W6, but has not yet produced cross-engine or
supported-platform release evidence
([harness evidence](test-results/2026-10-04-verification-harness.md)).

The current exact paired W2 random durable-load profile passes its narrow
active-throughput comparison. With a 64 MiB durable write buffer, WSLDB reached
524,189 operations/s versus freshly rebuilt RocksDB 11.8.1 at 491,990
operations/s in the same alternating session, a 6.55% WSLDB lead. This is not
a general win: RocksDB's median maximum durable-batch latency was 46.309
milliseconds versus WSLDB's 297.086-millisecond checkpoint tail, WSLDB used
7.29% more peak RSS, and WSLDB used 1.23% more logical storage.

Deriving the complete WAL-record CRC32C from independently computed header and
payload checksums improved the causal W2 median by 4.37%. Buffered 256 KiB
checkpoint page writes previously improved it by 8.38% and reduced maximum commit latency
by 36.71%. A single-pass ordered version-map update improved W2 by 7.39%, reuse
of ordered membership results improved it by 9.68%, and the negative membership
index improved it by 8.34% while reducing peak RSS by 24.68%. Packing version
records and value bytes into contiguous arenas then reduced peak RSS by another
9.90% without a throughput regression. Lazily constructing the sharded point
index then improved W2 by 11.44% and reduced write-only peak RSS by 19.44%,
while the measured 64-reader throughput and p99 also improved after warm-up.
An allocation-light borrowed bulk loader reduced maximum checkpoint latency by
9.06%, but failed its joint acceptance gate with a 0.18% W2 throughput
regression and was completely removed.
A current-buffer negative filter improved W2 by 3.29% with byte-identical
output, but missed its preregistered 4% threshold and was also removed.
Combining that filter with direct single-allocation WAL encoding produced a
3.08% gain, still below its 4% compound gate, so that candidate was removed.
The final unified path combined those two mechanisms with allocation-light
checkpoint loading and passed both primary gates: median causal throughput
improved by 6.85% and maximum commit latency decreased by 12.27%.
The subsequent freeze-and-swap checkpoint passed a stricter final-code gate:
median active W2 throughput improved by another 12.64%, maximum commit latency
fell from 239.934 to 18.558 milliseconds, active-plus-drain time improved by
8.54%, and peak RSS grew by 5.85% within its frozen 10% guardrail. Database and
WAL output remained byte-identical. A final fresh paired session then measured
625,008 operations/s versus RocksDB's 508,787 and 17.958 versus 45.028
milliseconds maximum durable-batch latency. Those are respective WSLDB wins of
22.84% and 60.12% on the exact W2 batch-10,000 profile. WSLDB simultaneously
used 2.69 times RocksDB's peak RSS and 1.23% more logical storage, so the result
did not support a general superiority claim.
Streaming WAL snapshot publication and sharing immutable version storage across
the handoff then reduced causal W2 peak RSS by 35.59%, improved throughput by
1.15%, and reduced active-plus-drain time by 7.60%, with byte-identical main
and WAL output. A new clean paired session measured 640,883 operations/s for
WSLDB versus 538,271 for RocksDB and maximum durable-batch latencies of 18.501
versus 46.124 milliseconds. WSLDB's memory disadvantage fell from 2.69x to
1.74x, but its storage remained 1.23% larger.
Prefix-compressing sorted WAL keys then reduced total WSLDB storage by 1.86%
to 121,155,676 bytes, 0.65% below the same RocksDB reference. In its causal
A/B, active throughput improved by 1.71%, active-plus-drain time improved by
2.02%, peak RSS grew by 0.92%, and maximum commit latency fell by 5.16%. This
intentionally changed only the single unreleased, unversioned WAL format; its
committed decoder fixture was regenerated and the complete recovery gate
passed. A fresh alternating session after that change measured 620,753
operations/s versus RocksDB's 559,712 and maximum durable-batch latencies of
20.965 versus 46.004 milliseconds. WSLDB was 10.91% faster in the active
interval, used 0.65% less logical storage, and retained a 54.43% lower maximum
batch latency, while using 1.76 times RocksDB's peak RSS.
All earlier runtime optimizations retained byte-identical database and WAL
output; prefix encoding is the documented pre-release format change. WSLDB's
remaining workload, resource, cold-cache, and platform gaps still block any
general performance-superiority claim
([fresh WAL-prefix RocksDB evidence](test-results/2026-10-04-wal-prefix-rocksdb-w2.md),
[WAL prefix causal evidence](test-results/2026-10-04-wal-prefix-encoding-preregistration.md),
[fresh zero-copy RocksDB evidence](test-results/2026-10-04-zero-copy-rocksdb-w2.md),
[zero-copy causal evidence](test-results/2026-10-04-zero-copy-wal-snapshot-preregistration.md),
[freeze-and-swap RocksDB evidence](test-results/2026-10-04-freeze-swap-rocksdb-w2.md),
[previous fresh RocksDB evidence](test-results/2026-10-04-unified-rocksdb-w2.md),
[freeze-and-swap evidence](test-results/2026-10-04-freeze-swap-checkpoint-preregistration.md),
[unified-path evidence](test-results/2026-10-04-unified-buffered-write-path-preregistration.md),
[previous RocksDB evidence](test-results/2026-10-04-lazy-index-rocksdb-w2.md),
[lazy-index evidence](test-results/2026-10-04-lazy-point-index-preregistration.md),
[rejected bulk-loader evidence](test-results/2026-10-04-borrowed-bulk-loader-preregistration.md),
[rejected buffer-filter evidence](test-results/2026-10-04-buffer-key-filter-preregistration.md),
[rejected compound evidence](test-results/2026-10-04-buffer-filter-direct-wal-preregistration.md),
[packed-arena evidence](test-results/2026-10-04-packed-version-arena-preregistration.md),
[combined-CRC evidence](test-results/2026-10-04-crc32c-combine-preregistration.md),
[buffered-page evidence](test-results/2026-10-04-buffered-checkpoint-pages-preregistration.md),
[version-map evidence](test-results/2026-10-04-single-pass-version-map-preregistration.md),
[preflight evidence](test-results/2026-10-03-buffer-preflight-preregistration.md),
[membership evidence](test-results/2026-10-03-base-membership-filter-preregistration.md),
[CRC evidence](test-results/2026-10-03-slicing-by-8-crc32c.md)).

The current symmetric warm-cache five-second W5/W6 diagnostic and
operation-limited adapter conformance evidence are documented
[here](test-results/2026-10-04-current-sustained-smoke.md). WSLDB uses 17.6% to
17.7% less peak RSS, 0.36% to 0.42% less logical storage, and has much lower
warm point-read p99, but remains 10.0% behind RocksDB on W5 throughput and 7.6%
behind on W6. It also loses some durable-mutation tails. The diagnostic does
not satisfy the long-duration gate. A profiled main-handle-reuse candidate
failed its preregistered normalized throughput and tail gates and was removed
([negative evidence](test-results/2026-10-04-buffered-main-handle-preregistration.md)).
Removing the duplicate WAL-length preflight from buffered commits was also
rejected: it improved seven-run median W5 throughput by 1.92%, but reduced W6
throughput by 1.25% and increased W6 overwrite p99 by 5.10%. The candidate and
its candidate-only test were removed after byte-identical conformance passed
([negative evidence](test-results/2026-10-04-single-wal-length-check-preregistration.md)).
A stronger non-empty-WAL main-validation fast path won every paired process
and improved median W5 throughput by 3.98%, but its 2.70% W6 improvement missed
the preregistered 3% threshold. It passed byte-identical conformance and all
secondary tail, RSS, and storage guardrails, but was still removed rather than
accepted by rounding or changing the gate after measurement
([negative evidence](test-results/2026-10-04-nonempty-wal-main-validation-preregistration.md)).

A later adapter-parity audit found that these historical RocksDB W5/W6 runs
used `Delete` without the exact existence lookup performed by WSLDB's measured
`WriteTransaction::delete`. RocksDB reports a successful delete even for a
missing key, so the old adapter both lacked the same fail-closed invariant and
performed less point-read work in every delete/recreate cycle. The adapter now
performs a measured `Get` before `Delete`. All earlier sustained ratios remain
historical diagnostics only and cannot be promoted into the pinned release
matrix ([evidence](test-results/2026-10-05-sustained-delete-parity.md)).
The same audit also found that RocksDB encoded each selected key before its
operation timer while WSLDB encoded it inside the timer. WSLDB now pre-encodes
the key at the same boundary, and the shell contract pins that ordering for
both adapters. Earlier sustained timings remain historical for this additional
reason; the magnitude of the correction is deliberately unclaimed until the
external matrix is rerun
([evidence](test-results/2026-10-05-sustained-timer-parity.md)).
The audit then found that per-window reporting was charged to the following
active throughput window. This was especially unfavorable to WSLDB because it
also captures checkpoint activity every second. Both adapters now reset their
active clocks after reporting and maintenance, with source-order contract
coverage. Earlier ratios remain historical and no corrected result is inferred
([evidence](test-results/2026-10-05-sustained-window-accounting.md)).
Finally, the runner default had left WSLDB in direct copy-on-write mode while
RocksDB retained its implicit memtable. The corrected contract defaults to the
preregistered 64 MiB buffered profile, passes the same recorded positive value
to both adapters, and makes RocksDB's `write_buffer_size` explicit. Zero is
rejected rather than silently comparing different persistence architectures
([evidence](test-results/2026-10-05-sustained-write-buffer-parity.md)).

Exact successful buffered-commit instrumentation now accounts separately for
preflight and append-boundary validation, WAL encoding, positioned write,
stable-storage synchronization, and post-sync publication. A clean short W6
diagnostic attributed 90.011% to synchronization and 5.825% to preflight;
all other individual phases were at or below 2.224%
([phase evidence](test-results/2026-10-04-buffered-commit-phase-preregistration.md)).
The resulting compound preflight candidate combines direct singleton
point-index validation with a non-empty-WAL main-validation fast path. Its
corrected implementation improved three-run median short W6 throughput by
6.52%, reduced mean preflight by 69.33%, improved both mutation p99 values,
and preserved RSS, storage, logical hashes, and exact persisted bytes. The
complete fault suite found and then closed an ambiguous-checkpoint publication
case before acceptance. This is accepted causal evidence, but it has not yet
been rerun against RocksDB or through the long frozen matrix
([candidate evidence](test-results/2026-10-04-compound-buffered-preflight-preregistration.md)).

A subsequent short routing diagnostic compared exact WSLDB WAL phase counters
with RocksDB 10.10.1's WAL histogram. Median WSLDB sync time was 4.716
milliseconds versus RocksDB's 4.907 milliseconds, and median active throughput
was 4.48% higher, but both sync and throughput changed sign between pairs.
WSLDB mutation p99 remained worse. The preregistered result is therefore
inconclusive and cannot replace the pinned RocksDB 11.8.1 release gate. It does
show that another CPU-only foreground optimization or an unmeasured change of
tree is not justified by this profile: the next measured routes are concurrent
group commit and buffered checkpoint/compaction policy
([diagnostic evidence](test-results/2026-10-04-homebrew-rocksdb-wal-timing-preregistration.md)).

Automatic foreground group commit now gives concurrent blind-write callers
their own consecutive transaction IDs while sharing naturally overlapping WAL
synchronizations. In the preregistered 16-thread mechanism diagnostic it used
328 syncs for 4,096 transactions and reached 1,973.986 transactions/s versus
185.457 transactions/s for individual sequential commits, a 10.644-times
gain with identical logical contents and WAL bytes. Its foreground leadership
is drop-guarded so an unexpected leader unwind permanently fails the affected
coordinator closed; queued and future calls return a typed error instead of
hanging or continuing from potentially ambiguous in-memory WAL state
([liveness evidence](test-results/2026-10-05-group-leader-unwind.md)). The
pending coordinator queue now uses constant-time front removal. Bounded group
draining and disconnected-waiter handoff no longer shift the complete pending
tail while holding the coordinator mutex; FIFO and failure propagation remain
covered
([queue evidence](test-results/2026-10-05-automatic-group-queue.md)). This
passes the local mechanism gate but still requires a frozen concurrent RocksDB
comparison on the supported platform matrix
([evidence](test-results/2026-10-04-automatic-group-commit-preregistration.md)).

A short five-pair diagnostic against Homebrew RocksDB 10.10.1 subsequently
showed that grouping has real comparative potential: WSLDB won every pair,
with 42.24% higher median throughput and 34.95% fewer WAL syncs. The complete
gate nevertheless failed because WSLDB used 2.32 times the maximum RSS. The
measured excess comes from eagerly sizing the write-buffer membership filter
for the configured 1 GiB limit rather than the 4,096 keys actually present.
The result is retained as negative evidence and routes the next causal work to
a bounded growing filter; it is not a pinned-version or general performance
win ([evidence](test-results/2026-10-04-concurrent-rocksdb-group-preregistration.md)).

The first bounded growing-filter candidate confirmed the memory diagnosis:
median maximum RSS fell 83.89%, from 24,920,064 to 4,014,080 bytes, while exact
logical and persisted output remained unchanged. It nevertheless reduced
median concurrent throughput by 5.17%, beyond its frozen 3% allowance, and won
only one of five causal pairs. The implementation was therefore removed in
full. A different memory design must avoid adding work to the commit preflight
and receive a new contract before implementation
([negative evidence](test-results/2026-10-04-growing-buffer-filter-preregistration.md)).

A guarded successor keeps the same geometric pre-sync rebuild but skips exact
new-key accounting while the current filter has capacity. It retained the
83.44% causal RSS reduction with only a 1.48% median-throughput change and
passed the complete correctness suite. In a fresh five-pair comparison against
Homebrew RocksDB 10.10.1, WSLDB won every concurrent strict-durability pair,
with 25.87% higher median throughput, 29.23% fewer WAL syncs, and 62.71% lower
peak RSS. WSLDB still wrote 62.72% more WAL bytes and used 45.71% more final
storage in this small empty-database profile. The accepted result establishes
a narrow scalable-write win, not pinned-version, Linux, or general superiority
([evidence](test-results/2026-10-04-guarded-growing-filter-preregistration.md)).

The current 30-minute W5 and W6 workloads also complete on APFS with 1,195,318
and 1,241,293 operations respectively. Each retained 29 periodic full
verifications plus a final verification, an exact 1,000,000-entry result, and
successful independent reopen/replay. W5 active throughput was 674.633
operations/s, final drain was 191.088 milliseconds, and whole-run overwrite
and delete/recreate p99 were 13.800 and 23.245 milliseconds. W6 active
throughput was 700.324 operations/s, final drain was 54.963 milliseconds, and
the corresponding p99 values were 13.020 and 22.024 milliseconds
([W5 evidence](test-results/2026-10-04-current-w5-30m.md),
[W6 evidence](test-results/2026-10-04-current-w6-30m.md)). These close
single-process current-code duration checks only. The frozen repeated W5/W6
comparison against RocksDB, cold-cache coverage, and supported-platform runs
remain open.

A subsequent single 30-minute RocksDB 11.8.1 W5 process measured 661.587
active operations/s versus WSLDB's 674.633, while WSLDB used 23.07% less peak
RSS and drained 94.39% faster. WSLDB nevertheless used 21.55% more final
logical storage and had 29.77% and 23.95% higher overwrite and delete/recreate
p99. The observed throughput result was dominated by one 278.778-second
RocksDB delete/recreate stall; excluding that one window gives RocksDB 784.456
operations/s. The stall's engine-versus-host cause is not isolated, so this
diagnostic supports neither a general throughput win nor post-hoc removal of
the stall
([evidence](test-results/2026-10-04-current-rocksdb-w5-30m.md)). Repeated
alternating runs remain mandatory.

The corresponding single 30-minute Zipf-1.2 W6 RocksDB diagnostic was stable:
it completed 1,380,095 operations with 29 successful periodic verifications,
no operation window longer than two seconds, and a successful independent
reopen. RocksDB reached 777.302 active operations/s versus WSLDB's 700.324, so
WSLDB was 9.90% slower on active throughput. WSLDB retained materially better
point-read p99, peak RSS, and final-drain results, but its overwrite and
delete/recreate p99 were 25.34% and 22.61% higher and its final logical storage
was 3.26% larger. This narrows the current performance deficit to the durable
mutation and ordered-verification paths on this profile; it does not replace
the required alternating repeated comparison
([evidence](test-results/2026-10-04-current-rocksdb-w6-30m.md)).

The same diagnostic attributes 90.8% to 93.1% of current buffered commit time
to the mandatory WAL sync. Explicit and automatic groups now write one compact,
atomically recovered physical frame and confirm it with one shared sync. Its
completion, partial-write rollback, unknown-outcome, crash-recovery, checkpoint,
corruption, and bounded resource behavior have focused coverage
([resource-bound evidence](test-results/2026-10-05-group-resource-bounds.md)).
A short alternating APFS
comparison against Homebrew RocksDB 10.10.1 measured 42.59% higher median
throughput, 31.72% fewer WAL syncs, 61.89% lower RSS, effectively equal WAL
bytes, and 9.36% smaller final storage. WSLDB won all five throughput pairs
([evidence](test-results/2026-10-04-compact-group-wal-preregistration.md)).
This closes only that concurrent strict-durability diagnostic. The sequential
W6 mutation-tail debt, pinned RocksDB 11.8.1 matrix, Linux repetition, and
filesystem power-loss gates above remain open.

A primary-source review of write-optimized trees, hybrid CoW/LSM layouts,
key/value separation, compaction scheduling, perfect hashing, and filesystem
journaling does not justify an unmeasured storage-core rewrite. The frozen
routing order first measures pinned W5/W6 phase and checkpoint correlation,
then the existing concurrent group contract; only a demonstrated data-movement
or checkpoint-interference bottleneck may open a separately preregistered core
experiment
([research routing](test-results/2026-10-05-durable-write-research-routing.md)).
