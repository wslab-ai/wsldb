# Chunked buffered maintenance preregistration

Date: 2026-10-04. This contract is committed after rejecting entry-granular
streaming and before implementing or measuring this candidate.

## Causal diagnosis

The rejected streaming candidate cut median compaction RSS by 80.75%, but made
wall time 59.58% worse. A single depth-first visitor removed repeated tree
seeks, so the remaining causal difference is scheduling: source reads and
destination page writes are interleaved for nearly every entry. The process
retired only about 2.5% more instructions than baseline but issued materially
more voluntary and involuntary context switches and accumulated more
concurrent WAL commits before catch-up.

RocksDB exposes multi-megabyte compaction readahead and bounds bytes per
compaction job rather than alternating tiny read/write units:
<https://github.com/facebook/rocksdb/wiki/Compaction>. This candidate borrows
that batching principle, not RocksDB code or its LSM layout.

## Frozen candidate

Baseline source commit:
`e1e32b25febf75849ab69ce8209a6c855805d6ea`.

- Traverse the physical B+ tree exactly once in key order.
- Merge the captured buffered overlay during that traversal.
- Accumulate at most 4 MiB of key/value payload or 32,768 entries, whichever
  comes first, then feed the chunk to the existing bulk loader in order.
- One legal value may exceed 4 MiB; the bound is therefore one maximum value
  plus the fixed chunk, bounded WAL overlay, page cache, and loader frontier.
- Apply the same chunked visitor to direct and buffered compaction. Backup,
  named compaction, active replacement, verification, delta catch-up,
  durability ordering, and the persistent format remain semantically
  unchanged.

Four MiB is fixed before measurement: it is large enough to avoid thousands of
read/write phase switches in the 100-byte-value profile while staying far below
the rejected all-live-snapshot allocation.

## Frozen acceptance gates

Reuse one 100,000-key random-load fixture and run three new alternating
fresh-process baseline/candidate pairs. The candidate is accepted only if:

- median maximum RSS is at least 25% below baseline;
- median compaction wall time regresses by no more than 15%;
- median publication-lock pause and concurrent commit p99 regress by no more
  than 25%;
- all runs preserve the exact logical snapshot, report only the expected stale
  generation conflict, empty the WAL, and reduce storage by at least 5%;
- overlay, maximum-key, empty-physical-tree, backup, named compaction, active
  compaction, freeze-and-swap, catch-up, crash, and injected-I/O tests pass;
- a deterministic 1,000-operation no-maintenance control is byte-identical to
  baseline;
- formatting, complete all-target all-feature tests, focused release tests,
  all-target check, Clippy with warnings denied, and `git diff --check` pass.

A failed candidate is removed. A passing result establishes bounded-memory
maintenance only; it does not establish automatic scheduling or RocksDB
superiority.

## Result

Rejected. No source commit contains this candidate.

The fixed 4 MiB/32,768-entry chunks solved the read/write interleaving cost and
retained most of the memory reduction, but the frozen publication-pause gate
failed.

| Median | Baseline | Chunked candidate | Change | Gate |
| --- | ---: | ---: | ---: | ---: |
| Maximum RSS | 25,640,960 B | 10,387,456 B | -59.49% | at most -25% |
| Compaction wall time | 2.050 s | 1.928 s | -5.93% | at most +15% |
| Publication-lock pause | 50.714 ms | 85.998 ms | +69.58% | at most +25% |
| Concurrent commit p99 | 11.822 ms | 11.039 ms | -6.63% | at most +25% |

All six runs preserved verification, emptied the WAL, reported only the
expected generation conflict, and reduced storage by more than 96%. The pause
failure is nevertheless decisive. The candidate is retained only as the basis
for a separately preregistered pre-publication catch-up experiment: apply the
large captured delta to the still-unpublished replacement without blocking
writers, then take the final lock for only commits that arrived during that
catch-up.

Raw binaries and logs are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-chunked-buffered-maintenance`.
