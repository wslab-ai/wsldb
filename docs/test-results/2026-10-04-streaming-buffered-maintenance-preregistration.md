# Streaming buffered maintenance preregistration

Date: 2026-10-04. This contract is committed before implementation or
candidate measurement.

## Problem and research route

Buffered backup, `compact_to`, and active compaction currently call
`FileStorageEngine::scan`, materializing every live key and value in one
`Vec` before the bulk loader starts. The 100,000-key delta-catch-up diagnostic
already reached 25,608,192 bytes of peak RSS. More importantly, memory grows
with the complete database, so a valid multi-million-key database can exhaust
the process while trying to recover space.

The candidate applies the same bounded-work principle used by streaming
compaction systems without changing WSLDB into an LSM tree:

- RocksDB assigns compaction work from size-based scores and bounds individual
  jobs rather than materializing the database in memory:
  <https://github.com/facebook/rocksdb/wiki/Compaction> and
  <https://github.com/facebook/rocksdb/wiki/Leveled-Compaction>
- O'Toole, Nettles, and Gifford, “Concurrent Compacting Garbage Collection of
  a Persistent Heap”, SOSP 1993, describes building a compact replica while a
  log captures concurrent transactions:
  <https://www.cs.cmu.edu/afs/cs/project/venari/www/sosp93.html>
- Dayan et al., “Monkey: Optimal Navigable Key-Value Store”, separates total
  merge work from its bounded scheduling and de-amortization:
  <https://scholar.harvard.edu/files/stratos/files/monkeykeyvaluestore.pdf>

These sources motivate bounded, concurrent reconstruction. WSLDB retains its
copy-on-write B+ tree, WAL, recovery protocol, and independently implemented
bulk loader.

## Frozen candidate

Baseline source commit:
`e1e32b25febf75849ab69ce8209a6c855805d6ea`.

For one captured buffered snapshot:

1. Under the existing stable buffer transition protocol, capture its physical
   checkpoint and one ordered overlay containing only the newest visible value
   or tombstone for each key changed after that checkpoint. A pending
   freeze-and-swap checkpoint may contribute more than one bounded buffer
   layer; newer layers override older ones.
2. Stream the immutable physical B+ tree in batches of 256 entries. Merge its
   ordered entries with the ordered overlay directly into the existing
   bottom-up bulk loader. Tombstones suppress physical values and never reach
   the output.
3. Validate the exact logical entry count before publishing. Keep full
   destination verification, sync ordering, delta catch-up, rename, and WAL
   retirement unchanged.
4. Use this source for buffered backup, named compaction, and active
   compaction. Direct-copy backup and direct-tree compaction remain unchanged.

The resulting working-memory bound is proportional to one physical scan batch,
the explicitly configured durable-buffer generations, the page cache, and the
bulk-loader frontier. It must not be proportional to all live database values.
There is no persistent-format or public-API change.

## Frozen acceptance gates

The causal benchmark reuses one 100,000-key random-load fixture and runs three
alternating fresh-process pairs. Each clone receives the same seed buffered
commit before active compaction and the same deterministic concurrent overwrite
loop. Baseline and candidate binaries are built separately from the commits
being compared.

The candidate is accepted only if:

- median maximum RSS falls by at least 25% from the baseline;
- median compaction wall time regresses by no more than 15%;
- median publication-lock pause and concurrent commit p99 regress by no more
  than 25%;
- every run preserves exact entry count and logical hash, reduces storage by at
  least 5%, leaves an empty WAL, and reports only the expected stale-generation
  conflict;
- deterministic tests cover overlay ordering, older snapshot visibility,
  tombstones, keys present only in the physical tree or buffer, maximum-width
  keys, and an empty physical tree;
- buffered backup, named compaction, active compaction, freeze-and-swap,
  concurrent catch-up, crash, and injected-I/O suites pass;
- a separate 1,000-operation no-maintenance control preserves operation count,
  entry count, logical hash, main-file SHA-256, and WAL SHA-256;
- formatting, complete all-target all-feature tests, release compaction tests,
  all-target check, Clippy with warnings denied, and `git diff --check` pass.

A passing result establishes a bounded-memory maintenance path. It does not
establish RocksDB superiority, automatic scheduling, or a bound smaller than
the configured WAL generations and page cache.

## Result

Rejected. No source commit contains this candidate.

The first implementation performed a new root-to-leaf range seek for every
256 entries. It reduced memory strongly but made compaction about 37% slower in
the initial diagnostic. Replacing those repeated seeks with one depth-first
visitor retained the memory result, but the frozen three-pair rerun still
failed the wall-time gate. Interleaving source reads with destination writes at
entry granularity increased I/O scheduling and allowed more concurrent WAL
commits to accumulate before catch-up.

| Median | Baseline | Streaming visitor | Change | Gate |
| --- | ---: | ---: | ---: | ---: |
| Maximum RSS | 25,624,576 B | 4,931,584 B | -80.75% | at most -25% |
| Compaction wall time | 1.938 s | 3.093 s | +59.58% | at most +15% |
| Publication-lock pause | 32.922 ms | 40.666 ms | +23.52% | at most +25% |
| Concurrent commit p99 | 13.918 ms | 14.377 ms | +3.30% | at most +25% |

Every run preserved verification and reduced storage by more than 96%, and the
memory hypothesis was confirmed. The wall-time failure is decisive, so the
entry-granular implementation is not accepted. Raw binaries and logs are
retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-streaming-buffered-maintenance`.

The result routes a separate preregistered candidate to a bounded multi-megabyte
read phase: retain one traversal and bounded memory, but feed the bulk loader in
large chunks so source reads and destination writes are not alternated for
every entry.
