# Backup and restore

`Database::backup` captures the latest durable `Head` while holding the local
commit mutex and OS file lock. It then releases both locks and copies the fixed
immutable page range belonging to that snapshot. Concurrent commits may append
new pages but cannot change any page visible to the captured root. The public
database-state lock is likewise held only long enough to clone the storage
generation; it is not retained during the copy or verification.

The destination is written under a temporary name in the target directory. It
receives a synthetic metadata page for exactly the captured root, is synced,
reopened logically, and fully verified without using a page cache. Verification
includes a streaming SHA-256 digest of the canonical ordered key/value stream.
Only then is the file hard-linked to the requested final path. The hard link is
a no-clobber atomic publish: an existing target produces an error and is never
modified. Directory entries are synced on Unix before the temporary name is
removed.

Restore currently means opening the resulting database file normally. A backup
contains a complete database snapshot and is not tied to the source path.

When a durable write buffer is active, backup merges the immutable checkpoint
and captured ordered WAL overlay into the same bottom-up builder used by
compaction. It stages at most 4 MiB or 32,768 entries rather than materializing
every live value. The resulting backup is a standalone page-tree file and
never depends on copying a WAL sidecar. Before publishing it, the operation
requires the destination's fully verified entry count and canonical logical
hash to match the ordered merged source stream. The same pass fully validates
the physical checkpoint structure, so the rebuild cannot silently normalize a
checksum-valid tree with invalid separators, depth, or shared pages.

A deterministic subprocess test pauses both backup implementations after their
source snapshot is fixed, commits a new source transaction, and then resumes
publication. The source advances while the backup retains the earlier complete
logical hash and excludes the concurrent mutation.

This algorithm depends on the current append-only rule that committed page IDs
are never reused. Page reclamation or online compaction must first add a reader
epoch that also protects active backup snapshots.
