# WSLDB

WSLDB is an experimental embedded key-value database written in safe Rust. It
uses an incremental copy-on-write B+ tree, immutable read views, one serialized
writer, checksummed pages, and alternating metadata pages.

No WSLDB release has been published. The current source is for evaluation and
correctness work; it is not production-ready and its file format is not stable.

```rust
use wsldb::Database;

let path = std::env::temp_dir().join("example.wsldb");
let db = Database::open(&path)?;

let mut write = db.write()?;
write.put(b"answer", b"42")?;
write.commit()?;

let read = db.read()?;
assert_eq!(read.get(b"answer")?, Some(b"42".to_vec()));
let first = read.range(b"a", Some(b"z"), 10)?.next();
assert_eq!(first, Some((b"answer".to_vec(), b"42".to_vec())));
# drop(read);
# drop(db);
# std::fs::remove_file(path)?;
# Ok::<(), wsldb::Error>(())
```

## Current guarantees

- Keys are maintained in deterministic bytewise order in a multi-level B+ tree.
- Readers retain an immutable root and do not block a writer after starting.
- A stable sidecar lock serializes cooperating writers across processes, even
  while the database inode is atomically replaced. Database paths are made
  absolute and existing symbolic links resolve to the same sidecar; Unix
  database files with multiple hard links are rejected fail-closed.
- The `.lock` sidecar is part of the live database identity and must not be
  deleted or renamed while handles are open. On Unix, an existing handle
  compares its open descriptor with the current path after every OS-lock
  acquisition. An unlinked, renamed, or replaced sidecar fails before database
  I/O; applications must reopen the database.
- Main, lock, and WAL paths must resolve to regular files. Symbolic-link
  sidecars and multiply hard-linked sidecars are rejected before recovery or
  publication, preventing redirects into unrelated files or independent
  writer-lock domains.
- On Unix, buffered commits also compare their open WAL descriptor with the
  current `.wal` path before append. A removed, renamed, or replaced WAL makes
  ordinary, explicit-group, and automatic-group commits fail with a write
  conflict until refresh or reopen installs the current generation. Flush,
  backup, operational verification, and compaction fail under the same split
  identity instead of publishing from mixed generations.
- Refresh is monotonic for a live handle: it never installs a transaction ID
  lower than the currently observed durable snapshot. If a nonempty WAL is
  moved away while its main-file checkpoint is unchanged, refresh returns a
  write conflict without creating an empty replacement sidecar or discarding
  the in-memory committed view. A replacement generation at the same
  transaction ID is accepted only if full verification produces the same
  entry count and logical hash.
- A commit writes and syncs new immutable pages before publishing an
  alternating, checksummed metadata page.
- An opt-in durable write buffer appends one checksummed transaction record per
  commit. At the configured byte threshold it freezes the current generation
  without copying and prepares its page-tree checkpoint in the background;
  foreground commits continue in a bounded suffix. Recovery accepts only a
  continuous transaction chain. Opening without buffering still recovers an
  existing WAL, but keeps group commit disabled and returns to direct CoW after
  the WAL is materialized by `flush` or checkpoint.
- `Database::commit_group` appends ordered `WriteBatch` records in one WAL
  write and confirms them with one sync. It is explicit rather than
  delay-based, so applications control the throughput/latency tradeoff.
- On Unix, explicit `flush` atomically consolidates an outstanding transaction
  chain into one sorted snapshot record containing only the latest mutation per
  key. Snapshot bytes are streamed through bounded staging memory, while
  immutable keys and version storage are shared safely across the generation
  handoff. Oversized snapshots fall back to a page-tree checkpoint.
- Opening validates both metadata slots and the selected root, and can fall
  back from a torn or corrupt newest root. The default fast path validates
  deeper pages lazily as they are read. Applications that must reject any deep
  page or overflow corruption before receiving a handle can enable
  `DatabaseOptions::verify_on_open(true)` for a complete logical scan. With no
  pre-existing WAL, strict open can fall back to the newest older snapshot that
  passes the scan. It never falls back across a nonempty WAL and never rewrites
  the damaged generation. The older snapshot is a read-only recovery view:
  writes and active maintenance return `WriteConflict`, while `backup` and
  `compact_to` can export it into a new verified database if the source
  generation remains unchanged.
- First creation writes and syncs complete metadata in a temporary inode before
  atomically publishing the database name and syncing its parent directory.
  An interrupted creation therefore never exposes a partial final file.
- Corruption without a valid metadata/snapshot pair fails closed.
- A failed commit distinguishes a definite pre-publication I/O error from
  [`Error::CommitOutcomeUnknown`], which requires refresh or reopen before an
  application decides whether to retry.
- Backup, compaction, and buffer consolidation similarly distinguish a definite
  pre-publication error from [`Error::PublicationOutcomeUnknown`] after a
  verified file may already have been linked or renamed.
- Values up to 64 MiB use checksummed overflow page chains.
- `MAX_KEY_BYTES` and `MAX_VALUE_BYTES` expose the mutation admission limits
  without requiring applications to duplicate format constants.
- `Database::verify` validates the complete reachable tree and all values.
- A configurable bounded sharded LRU cache retains decoded pages and exposes
  hit/miss counters; a 64-page, eight-way set-associative per-thread L1 removes
  shared-lock traffic for hot tree levels.
- Bounded ordered range scans prune subtrees outside the requested key interval;
  `ReadTransaction::range_after` provides exclusive continuation keys for
  duplicate-free pagination within one immutable snapshot.
- Online backup captures a durable root, copies immutable pages without holding
  the writer lock, verifies the result, and atomically publishes the target.
- Snapshot compaction rebuilds live entries bottom-up into densely packed pages
  and atomically publishes a verified, standalone database file.
- Buffered backup and compaction merge the immutable checkpoint with its
  ordered WAL overlay in bounded 4 MiB chunks rather than materializing every
  live value. Active compaction hands off a byte-identical WAL generation
  before replacing the main file, so concurrent commits survive without B+
  tree catch-up under the publication lock.
- `Database::compact_file` atomically rotates a verified compact generation at
  the active path on Unix. Existing readers remain valid and stale writers fail
  with a conflict.
- `Database::compact` performs the same rotation while keeping that database
  handle usable for later reads and writes.
- `Database::refresh` moves a stale handle to the newest durable generation and
  snapshot without changing read transactions that already exist.
- Operational stats expose physical/live/retired/tail bytes, tree shape, cache
  behavior, commit and sync timing, database write amplification, failed
  automatic checkpoints, and detected corruption.
- Lightweight checkpoint stats expose starts, currently active work,
  successful publication latency, written bytes, and automatic failures
  without scanning or verifying the tree; sustained workloads mark every
  one-second window containing checkpoint activity.

Separately opened handles never retry a conflicting transaction implicitly.
The application can refresh and start a new transaction explicitly:

```rust
use wsldb::{Database, Error};

let path = std::env::temp_dir().join(format!("wsldb-refresh-{}.wsldb", std::process::id()));
# let _ = std::fs::remove_file(&path);
let first = Database::open(&path)?;
let second = Database::open(&path)?;
let mut stale = second.write()?;
stale.put(b"stale", b"not-published")?;

let mut current = first.write()?;
current.put(b"current", b"published")?;
let committed = current.commit()?;
assert!(matches!(stale.commit(), Err(Error::WriteConflict)));

assert_eq!(second.refresh()?, committed);
assert_eq!(second.read()?.get(b"current")?, Some(b"published".to_vec()));
# drop(second);
# drop(first);
# std::fs::remove_file(&path)?;
# let mut lock_name = path.as_os_str().to_os_string();
# lock_name.push(".lock");
# std::fs::remove_file(std::path::PathBuf::from(lock_name))?;
# Ok::<(), wsldb::Error>(())
```

Durable group commit is available when the write buffer is enabled:

```rust,no_run
use wsldb::{Database, DatabaseOptions, WriteBatch};

let path = std::env::temp_dir().join("grouped.wsldb");
let db = Database::open_with_options(
    path,
    DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
)?;
let mut first = WriteBatch::new();
first.put(b"first", b"one")?;
let mut second = WriteBatch::new();
second.put(b"second", b"two")?;
let final_transaction = db.commit_group(vec![first, second])?;
assert_eq!(final_transaction.value(), 2);
# Ok::<(), wsldb::Error>(())
```

Independent threads can instead call `commit_batch`. Calls that naturally
overlap share a WAL synchronization without a grouping timer; each call still
returns only after its own batch is durable and receives its own transaction
ID. See [Durable group commit](docs/group-commit.md) for ordering and failure
semantics.

## Known limitations

- The main page-tree generation grows append-only until `compact` or
  `compact_file` rotates it. Automatic page-tree compaction scheduling remains
  the embedding application's responsibility; the optional WAL threshold only
  schedules checkpoints into that generation.
- Durable write buffering is disabled by default. It substantially reduces
  random-write amplification, but an outstanding buffer increases reopen work,
  memory use, and point-read tail latency. Applications must select and measure
  a threshold for their workload rather than treating it as a universal win.
- Group commit requires durable buffering and explicit batches. WSLDB does not
  delay unrelated writers in the background to form groups automatically.
- Iteration and range pages return owned entries. Full iteration materializes
  the snapshot in memory; `range` and `range_after` bound each page by their
  explicit limit.
- Pages are not reused within a generation. This preserves old readers without
  a cross-process reader table at the cost of periodic whole-generation
  compaction.
- Linux and macOS are the officially supported platforms for the first
  release. Windows is expected to build and run the core database, but remains
  a best-effort, untested platform without production-support guarantees.
  Active-path atomic replacement is currently available only on Unix.
- Encryption, compression, incremental backup, and stable-format migration
  tooling are not implemented. Real power-loss validation has a resumable
  independent-oracle probe but has not completed the supported-filesystem
  matrix. Process-crash injection and persistence-boundary I/O fault injection
  are available as opt-in test features.
- The current file format remains subject to change until the first release.

See [Architecture](docs/architecture.md), [Durable group commit](docs/group-commit.md),
[Public API stability](docs/api-stability.md), [Roadmap](docs/roadmap.md), and
the [local release checklist](docs/release-checklist.md) for the storage and
validation plan. The current bytes are specified in
[File format](docs/file-format.md) and
[Real power-loss testing](docs/power-loss-testing.md).

## License

WSLDB is distributed under the MIT License, Copyright (c) 2026 Workstar Lab.
See the `LICENSE` file in the repository root.
