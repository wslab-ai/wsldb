# Snapshot compaction

`Database::compact_to` captures the latest durable root under the same short lock
used by backup. It then walks that immutable snapshot while concurrent commits
may continue appending to the source file. Buffered maintenance merges the
physical checkpoint with the captured ordered WAL overlay and stages at most
4 MiB or 32,768 entries before feeding the bulk loader. Memory is bounded by
that chunk, the configured buffer generations, the page cache, and the loader
frontier, plus one fallibly allocated reachability record per source page,
rather than by every live value. The fixed WAL-handoff copy buffer is also
allocated fallibly before any handoff bytes are written, so memory pressure
returns a typed error while the original main file and WAL remain
authoritative. The public database-state lock is released after cloning the
storage generation and is not held for the rebuild.

Live ordered entries are fed into a bottom-up bulk loader. The loader fills each
leaf once, writes overflow chains before their parent leaves, and constructs
branch levels from child minima. It does not replay historical transactions and
does not retain retired source pages. The resulting file preserves the source
transaction ID and logical contents while using a newly packed physical layout.
Its sole metadata head is placed in the slot selected by transaction parity,
so the first later commit publishes into the opposite slot and retains a
recoverable older head if that metadata write is partial.

Like backup, compaction writes a temporary file in the target directory, syncs
it, performs a full uncached structural and value verification, and atomically
hard-links it to a non-existing final path. `compact_to` never modifies the
source and is suitable for a separately named export.

The rebuild also hashes the canonical ordered key/value stream while it reads
the source. That source traversal verifies balanced depth, branch separators,
non-overlapping ranges, unique page reachability, overflow chains, and the
metadata entry count. Before publication, the verified destination must have
the same entry count and logical hash. These checks are part of the operation
rather than assertions left to callers, and apply to both direct snapshots and
the checkpoint-plus-WAL merge used by buffered backup and compaction.

`Database::compact_file` is the active-path variant. It builds and verifies a
new generation without blocking normal work, then takes the stable sidecar lock,
checks that the captured durable head is still current, and atomically renames
the replacement over the database path. Direct copy-on-write mode returns
`WriteConflict` if a concurrent commit advances the source. With durable write
buffering, the replacement keeps the captured logical transaction ID. While it
is built, the complete captured WAL prefix is copied and synced to a temporary
inode. Under the final locks, compaction appends only the newly committed WAL
tail, syncs it, renames this byte-identical WAL over the sidecar path, and then
renames the compact main file. The new WAL inode makes every old handle
detectably stale; the different compact root also distinguishes the physical
generation without inventing a logical transaction. Existing readers continue
through their open descriptors and remain valid. This operation currently
requires Unix rename-over-existing semantics.

The main-file generation lease validates its descriptor against the current
path only after the OS lock succeeds. This ordering closes a rename race in
which a waiter could open the old inode, block behind another compaction, then
acquire the old inode's lock after that compaction had already renamed a new
generation into place. Such a stale lease returns `WriteConflict` and cannot
protect or publish a second checkpoint.

The publication check is performed only after reacquiring the stable lock. A
deterministic subprocess test pauses after the replacement is fully ready and
publishes a competing commit. Direct mode rejects the stale replacement;
buffered mode carries the commit through the WAL handoff. Crash and I/O-failure
matrices cover WAL staging, tail copy, sync, rename and directory sync, followed
by main-file rename and directory sync. The original main plus either identical
WAL inode is valid before main publication; the compact main plus the same WAL
is valid afterwards.

`database.compact()` performs the same verified generation rotation and
atomically switches that `Database` handle to the new generation. Existing read
transactions retain the old generation; new reads and writes use the compacted
one. Separately opened handles still become stale and their writers return
`Error::WriteConflict`.

After a separately opened handle or process rotates the active generation,
`database.refresh()` atomically reopens the path and updates that handle's
snapshot. Read transactions created before the refresh remain pinned to their
old generation.

Crash injection covers both the fully synced replacement before publication and
the rename-before-directory-sync boundary. Recovery opens either the old or the
new fully verified generation. A process crash or an ambiguous error after a
replacement is complete can leave a `*.wsldb-replacement-*` file beside the
database; after confirming that no compaction process is running, operators may
remove such an orphan. A later compaction skips occupied replacement names and
never removes a collision it did not successfully publish, including an orphan
left by a previous process that reused the same PID and sequence.
The same rule applies to `*.wal.wsldb-handoff-*` files: they are never
authoritative until renamed over the WAL path.

The compaction report exposes the same canonical logical SHA-256 digest as full
source verification, so operators can compare contents independently of page
layout and physical size. `publication_pause` reports the interval for which
active compaction held the publication locks; standalone `compact_to` exports
report zero because they do not pause source writers.

If buffered commits are outstanding, compaction merges them with the checkpoint
tree before building the destination. Commits made later remain in the exact
WAL tail and are replayed over the compact checkpoint. The full WAL is durable
before main replacement, so recovery sees either the original checkpoint plus
that WAL or the compact checkpoint plus the same WAL. On reopen, an entirely
covered WAL may be retired; a newer suffix remains authoritative. The protocol
does not require an atomic rename of two files.
