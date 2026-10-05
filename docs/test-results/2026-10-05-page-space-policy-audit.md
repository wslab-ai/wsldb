# Page-space policy audit

Date: 2026-10-05. This is a source-and-evidence audit of the current page-space
policy. It does not introduce page reuse or report a new runtime measurement.

## Production policy

WSLDB does not reuse a physical page ID within a file generation. Direct CoW
commits allocate after the current file end, and buffered checkpoints also
build new pages after the selected physical boundary. A failed unpublished
commit may leave an uncommitted tail, but no later committed snapshot can
reinterpret an old physical page as a different node.

Space reclamation uses verified generation compaction rather than a free list.
Compaction writes all live entries to a separate temporary file, syncs and
fully verifies that file, and atomically replaces the active path only after
revalidating the source generation. The replaced inode remains open for read
transactions that captured it; later reads use the new dense generation.
Writers based on the old physical generation fail with `Error::WriteConflict`.

This policy deliberately avoids a cross-process oldest-reader table. Its cost
is append-only growth between compactions and copying all live entries during
generation rotation. Operational statistics expose physical, live, retired,
and uncommitted-tail bytes so the application can schedule compaction before a
configured storage limit is exceeded.

## Authoritative implementation

- `CommitContext` and `StreamingCommitContext` derive new physical page IDs
  from a monotonic next-page boundary and use checked increments.
- Replacing a node already stored in the file creates a new pending page;
  replacement in place is limited to virtual pages belonging to the same
  unpublished commit.
- Commit metadata records only the fully flushed reachable root and last page.
  Recovery never scans a free list and never publishes an uncommitted tail.
- `Database` read transactions retain an `Arc` to the captured storage
  generation plus its immutable head.
- Active compaction publishes a different inode and validates main-file and
  WAL identities under the stable sidecar lock.

The architecture and compaction specifications document the same ownership
contract. No format field, API, or decoder implies reusable physical page IDs.

## Existing verification

The latest all-target, all-feature run recorded in
`2026-10-05-ordered-buffer-node-preflight.md` executed 245 Rust tests plus five
opt-in benchmark entry checks on unchanged production code. Its included
coverage proves:

- an old read transaction remains readable after direct active compaction;
- an old read transaction remains readable after buffered active compaction;
- stale writers cannot publish over a rotated generation;
- a concurrent commit either prevents stale direct compaction publication or
  is retained through the buffered WAL handoff;
- crash and injected-I/O boundaries recover one complete old or new
  generation;
- compaction output is structurally verified and has the source logical hash.

The focused evidence remains in
`2026-10-03-compaction-publication-race.md`,
`2026-10-04-wal-handoff-compaction-preregistration.md`, and
`2026-10-05-main-file-lease-identity.md`.

## Decision

The current no-reuse plus atomic-generation-compaction design satisfies the
production page-space safety requirement. A free-page allocator is not a
first-release requirement and must not be added without a cross-process
oldest-reader protocol or an equally strong proof that no live reader can
observe reused storage.

Optional page reuse remains a future performance/space experiment. It requires
a preregistered workload showing that generation compaction is the measured
bottleneck, format and recovery design, crash injection at every allocation-
metadata boundary, multi-process reader/writer stress, and proof that cache
entries cannot alias newly assigned contents. Until those gates exist, adding
reuse would weaken the current immutable-page invariant.

This audit does not close the independent Linux, physical power-loss, or pinned
RocksDB release gates.
