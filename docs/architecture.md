# Architecture

WSLDB separates durable I/O from transaction policy and ordered data semantics:

```text
infrastructure/file_store -> application/StorageEngine -> domain/Head
                                      ^
                              application/Database
```

## File layout

The file begins with two fixed 4096-byte metadata slots. Remaining 4096-byte
pages contain immutable B+ tree branch, leaf, or overflow data. Every page stores
its creating transaction ID plus independent header and payload checksums. The
header checksum is bound to the physical page ID, so relocating an otherwise
valid page is detected.

Branch pages contain ordered separators and child page IDs. Leaf pages contain
ordered keys and either inline values or references to overflow chains. A leaf
stores its common key prefix once and independent suffixes for every entry; the
decoded tree and public API still use complete keys. Every child and overflow
continuation precedes its immutable parent in the file, making cycles and
forward references invalid by construction.

## Initial publication

A missing database is initialized under the stable sidecar writer lock. WSLDB
writes both empty metadata slots to a uniquely named temporary inode, syncs the
inode, hard-links it at the requested final path, and syncs the parent
directory. It then removes the temporary name and syncs the directory again.
The live file descriptor continues to reference the published inode.

An error before the link is a definite creation failure and leaves no final
database. An error after the link returns `PublicationOutcomeUnknown`; reopening
the final path determines whether publication succeeded. A process exit at any
instrumented boundary leaves the final path either absent or backed by complete,
checksummed metadata. Existing empty final files still fail closed rather than
being silently reinitialized.

## Commit protocol

1. An OS lock on a stable `<database>.lock` sidecar serializes cooperating
   writers across processes and across active-file replacement. The database
   path is canonicalized before any sidecar is opened, so relative and
   symbolic-link aliases converge on the same lock. Unix opens reject multiply
   hard-linked database files because path-based sidecars cannot establish one
   lock identity for distinct hard links. Before that check, recovery removes
   only reserved initialization-temporary names that reference the exact same
   device and inode, covering a crash after no-clobber publication but before
   temporary-name cleanup.
   The lock sidecar remains part of the live database identity and must never
   be manually deleted or renamed. On Unix, every acquired long-lived sidecar
   descriptor is compared with the current path after the OS lock succeeds. An
   unlinked, renamed, or replaced sidecar fails before database I/O; a stale
   handle must be reopened.
   Lock and WAL sidecars must be regular files with one directory link;
   symbolic links and multiply hard-linked sidecars fail before recovery or
   publication. The main path likewise must resolve to a regular file.
2. The writer rereads the durable head and rejects a stale base transaction.
   On Unix, a direct-mode commit also compares the current main-file device and
   inode with the descriptor pinned by the handle. This closes the equal-head
   case: an external byte-identical replacement still requires `refresh()` and
   cannot make a commit write one inode while subsequent reads use another.
3. Mutations create candidate copy-on-write paths in a transaction-local page
   buffer; splits create new immutable candidates.
4. Only candidates reachable from the final root are remapped child-first,
   written, and synced. Superseded intermediate candidates never reach disk.
5. The metadata slot selected by `txid % 2` publishes the new root.
6. The metadata page is synced before the file lock is released.
7. Only then is the new head published to readers in the current process.

An all-put transaction whose keys are beyond the committed maximum takes a
specialized construction path. It reads the right spine once, packs the new
leaf and branch pages in order, and leaves every untouched subtree shared with
the base root. The publication and sync protocol is identical to the general
mutation path. This is a construction optimization, not a weaker durability
mode.

On open, both metadata slots and referenced roots are validated. The valid head
with the highest transaction ID wins. A partially written newest commit leaves
the previous root usable; unreferenced pages are harmless until compaction.

## Optional write buffer

`DatabaseOptions::verify_on_open(true)` adds a complete reachable-tree and
overflow-value verification after metadata, root, and WAL recovery. It makes
open proportional to live database size; the default fast path keeps deep page
validation lazy without returning unchecked bytes from a read. When the WAL
was empty before recovery, strict open may select the newest older metadata
snapshot that also passes full verification. A nonempty WAL disables this
fallback so recovery cannot discard newer durable transactions. The corrupt
slot remains on disk, making the older snapshot a read/recovery view rather
than an automatic repair. Mutating and active-maintenance operations reject
that view. `backup` and `compact_to` revalidate the physical source generation
under the cross-process lock, then copy or rebuild the verified recovery head
into a separately named database.

`DatabaseOptions::write_buffer_bytes` enables an append-only `<database>.wal`
sidecar. Each commit appends a deterministic, checksummed record, syncs that
file once, and only then publishes the new logical head in memory. Records form
a continuous transaction chain from the page-tree checkpoint. A torn physical
tail is truncated to the last complete record; a complete corrupt record or a
transaction gap fails closed.

An empty WAL sidecar is not considered durable merely because its directory
entry exists. Opening a new or existing empty sidecar synchronizes its parent
directory before buffered writes are enabled. If that barrier fails, the file
remains empty and the next open repeats the barrier; no WAL record can be
appended until one attempt succeeds. A nonempty WAL already follows a
successfully published sidecar and does not add this startup barrier.

Overlapping blind writes and explicit batch groups share one compact physical
record and one sync. Logical transaction IDs and entry counts remain separate,
but recovery validates the complete group before applying any member. A torn
group therefore contributes no logical prefix; older concatenated individual
records remain readable.

Before appending, the writer validates the complete transaction, resulting WAL
end offset, ordered-buffer state, and every affected point-index shard. The
ordered membership result used for entry-count maintenance is retained for the
point-index comparison rather than recomputed. The process-local writer mutex
prevents another index writer between that preflight and publication.
Consequently, after the WAL sync succeeds, applying the already validated
record and publishing its in-memory head have no ordinary error return. A
process stop at that boundary is recovered from the WAL.

Point reads use a 64-shard in-memory index, while scans merge the ordered
buffered mutations with the immutable checkpoint tree. Every read transaction
still observes its original transaction ID. Once the configured byte threshold
is reached, the complete buffered state is frozen without copying and a new
mutable suffix is installed. A worker constructs immutable checkpoint pages
while foreground commits continue appending to the same WAL. Main-file writes
hold a generation lease, and the worker's data sync is serialized with both
process-local commits and the cross-process writer lock so durability barriers
do not contend with WAL syncs. On Unix, a lease reopens and compares the
current device/inode after its blocking or non-blocking OS lock succeeds. A
descriptor opened before another process renames a compacted generation can
therefore never become a lease for the now-unlinked old inode.

Dropping the final owner of a buffered storage generation joins its checkpoint
worker. Destruction can therefore wait briefly for already-started checkpoint
I/O, but that work cannot continue as a detached thread after the final owner
returns. A prepared checkpoint that was not published remains unreachable; its
pages are harmless and later active-generation compaction reclaims them.

Publication revalidates the main-file generation and WAL boundary, publishes
the already synced metadata, switches current readers to the equivalent new
physical root, and releases the frozen generation before encoding the mutable
suffix as one deterministic WAL snapshot. Metadata precedes WAL replacement:
after a stop, recovery can therefore use either the old checkpoint with the
full WAL or the new checkpoint with the full WAL or complete suffix. A stale
external commit or file replacement prevents publication. Explicit `flush`
waits for pending preparation; foreground commits publish only at the bounded
suffix threshold. Readers opened before the freeze retain the old storage
generation and its in-memory versions.

A checkpoint built by the active process also carries an in-memory
probabilistic membership filter for its immutable keys. A negative result skips
an exact tree lookup when a later commit computes its entry count; every
positive result still consults the tree. The filter is constructed from all
keys in a new checkpoint and only accumulates later inserted keys, so stale
bits from deletes can cause extra reads but cannot hide an existing key. It is
not persisted: a reopened database safely uses exact tree lookups until that
process builds a new checkpoint.

An explicit `Database::flush` uses a cheaper path on Unix. It writes the latest
mutation for each buffered key as one sorted, checksummed snapshot record,
syncs it, atomically renames it over the WAL, and syncs the parent directory.
Length and CRC32C are calculated in a borrowed pass, then the existing record
encoding is emitted through a bounded staging buffer without a record-sized
byte vector. The replacement buffer clones only ordered map nodes and shares
immutable keys, its append-only version arena, and any initialized point index;
transaction-ID visibility keeps readers pinned to older snapshots isolated.
The old transaction chain and the new snapshot record describe the same logical
head, so recovery at every rename boundary is unambiguous. Existing readers
retain the old descriptor and full in-memory version history; new readers use
the compact record. A separately opened handle bound to the replaced WAL inode
must refresh before writing. If one record cannot contain the snapshot, or
atomic replacement is unavailable, flush uses the page-tree checkpoint path.
Because neither consolidation nor a checkpoint reuses primary page IDs, the
replacement storage handle shares the existing decoded-page cache. A true
generation rotation still creates a new cache because its physical page IDs
have different ownership.

If a process stops during checkpointing, recovery either replays the complete
durable WAL over the prior checkpoint or recognizes the already published
checkpoint and discards the stale WAL. The direct page-tree commit path remains
the default because outstanding buffered records trade lower random-write
amplification for additional reopen work, memory, and point-read tail latency.

## Transaction model

Read transactions retain an immutable `Head`, so they remain stable across later
commits. A mutex serializes writers within one handle and an OS lock serializes
independent handles and processes. A failed durable commit never publishes its
candidate root.

Deletes apply the same copy-on-write rule. A child below half a page is merged
with an adjacent sibling when the combined node fits; otherwise both siblings
are redistributed by encoded byte size. Unary branches collapse recursively,
including the root.

Pages are never reused within a generation. Atomic generation rotation keeps
old file descriptors readable while new opens use a densely packed file; any
writer based on the prior physical head fails its durable-head comparison. This
avoids requiring a cross-process reader table, at the cost of copying all live
entries during rotation.

The application state stores the active storage handle and `Head` under one
lock, so an in-process generation switch cannot pair a root with the wrong file
descriptor. Existing read transactions own the old storage handle. `refresh`
checks the current path under the stable sidecar lock and atomically installs
the new head for later transactions. If the page-tree checkpoint and WAL inode
are unchanged, a buffered handle validates and replays only complete records
after its prior WAL end; a torn suffix is truncated before publication. A
checkpoint, WAL replacement, or generation rotation falls back to reopening the
complete current storage pair. Historical in-memory versions keep existing read
transactions on their original snapshot in either case.

## Read path

Reads validate a page before placing its decoded node in a configurable bounded
sharded LRU cache. Page IDs are never reused in the current format, so a cached
immutable node is valid for every snapshot that can reference it. Cached nodes
are shared by `Arc`; only a mutation path takes an owned copy. Cache capacity
and sharded hit/miss counters are exposed through the public API; capacity zero
bypasses the cache. Hits take a shard's shared read lock and periodically update
an atomic access stamp. Insertions and approximate LRU eviction take only the
target shard's exclusive lock. Each reader thread also retains at most 64
decoded pages in an eight-way set-associative L1. A shard generation invalidates
those slots after any insertion or eviction, and the additional bound is 64
pages per thread outside the shared-cache residency counter.
