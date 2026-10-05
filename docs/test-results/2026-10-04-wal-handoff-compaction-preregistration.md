# WAL-handoff buffered compaction preregistration

Date: 2026-10-04. This contract is committed after rejecting entry streaming,
fixed-size chunking alone, and optimistic B+ tree catch-up, and before this
candidate is implemented or measured.

## Causal diagnosis

The accepted buffered active-compaction path materializes every logical value,
then applies every commit made during reconstruction to the replacement B+ tree
while holding the publication locks. Entry streaming bounded memory but made
the reconstruction 59.58% slower. Fixed 4 MiB chunks recovered wall time and
reduced RSS by 59.49%, but its larger accumulated tree catch-up made the
publication pause 69.58% worse. Moving one tree catch-up round before the final
lock still regressed publication pause by 33.47% and foreground commit p99 by
40.98%.

The next candidate removes tree catch-up from publication rather than adding
more rounds. A compacted tree at logical transaction `T` is already a complete
checkpoint. The original WAL remains a valid recovery chain for both the old
physical checkpoint and `T`: recovery skips records covered by the selected
checkpoint and replays only records newer than it. A byte-identical WAL on a
new inode can therefore invalidate stale handles before the main-file rename
without introducing an incompatible crash state.

This is the same general log-assisted replica-construction principle described
by O'Toole, Nettles, and Gifford, “Concurrent Compacting Garbage Collection of
a Persistent Heap”, SOSP 1993:
<https://www.cs.cmu.edu/afs/cs/project/venari/www/sosp93.html>. WSLDB keeps its
own COW B+ tree, WAL encoding, recovery rules, and bulk loader.

## Frozen protocol

Baseline source commit:
`e1e32b25febf75849ab69ce8209a6c855805d6ea`.

1. Capture buffered logical snapshot `T`, its WAL boundary, physical checkpoint,
   and exact source byte count under the existing commit and sidecar locks.
2. Rebuild `T` with one ordered traversal and at most 4 MiB or 32,768 entries of
   staged key/value payload. The replacement keeps transaction ID `T`; its new
   root and last-page identity still reject a writer based on the old `Head`.
3. Concurrent WAL commits remain authoritative. Outside the publication lock,
   copy the captured complete WAL prefix into a temporary file and sync it.
4. Under the final commit and sidecar locks, refresh and validate the current
   WAL, append only bytes written after the captured boundary to the temporary
   copy, and sync it. Do not interpret or apply this tail to the replacement
   tree.
5. Atomically rename the complete WAL copy over the WAL path and sync the
   parent directory. This byte-identical new inode makes every stale process
   fail its existing WAL-identity check while the old main file remains fully
   recoverable.
6. Atomically rename the compacted main file over the main path and sync the
   parent directory. The new main plus the same complete WAL recovers every
   commit newer than `T`. Reopen and fully verify the current logical head after
   releasing publication locks.

The complete WAL is retained across both authoritative renames. On the first
reopen, the existing recovery rule may retire it only when the new checkpoint
already covers the complete WAL and there is no newer suffix; otherwise the
complete WAL remains for later snapshot consolidation or checkpointing.
Reported compacted bytes include both the compact main file and the WAL that
remains after recovery selection. No persistent encoding or public method
signature changes.

## Frozen correctness gates

The candidate is rejected unless all of the following hold:

- deterministic commits before and during reconstruction survive with their
  original transaction IDs, exact entry count, and exact logical hash;
- a stale handle opened before publication receives `WriteConflict`, while a
  refreshed handle remains writable;
- process termination after temporary-WAL writes and sync, WAL rename and
  directory sync, main rename and directory sync recovers either the complete
  old generation or the complete compact generation, never a mixture;
- injected WAL-copy, sync, rename, directory-sync, and main replacement
  failures preserve a reopenable committed state and return the documented
  definite or ambiguous error class;
- buffered backup, named compaction, active compaction, freeze-and-swap,
  recovery, corruption, multiprocess, and existing maintenance suites pass;
- exact committed WAL bytes are unchanged across the handoff renames; recovery
  may subsequently reduce them to zero only when the new checkpoint covers
  every record;
- source files remain at or below 700 physical lines and no `unsafe` is added.

## Frozen performance gates

Reuse the 100,000-key fixture and run three new alternating fresh-process
baseline/candidate pairs with the same deterministic concurrent overwrite
worker. Accept only if:

- median maximum RSS is at least 25% below baseline;
- median compaction wall time regresses by no more than 15%;
- median publication-lock pause and concurrent commit p99 regress by no more
  than 25%;
- every run preserves the exact logical snapshot, reports only the expected
  stale-generation conflict, and reduces total main-plus-WAL storage by at
  least 5%;
- a deterministic 1,000-operation no-maintenance control preserves operation
  count, entry count, logical hash, main-file SHA-256, and WAL SHA-256;
- formatting, complete all-target all-feature tests, focused release tests,
  all-target check, Clippy with warnings denied, and `git diff --check` pass.

A passing result establishes bounded-memory buffered maintenance and bounded
publication work for this profile. Retaining the complete WAL means it does not
establish minimal recovery time, automatic compaction scheduling, or general
superiority over RocksDB.

## Result

Accepted. The implementation replaces whole-snapshot materialization with one
ordered physical traversal plus a captured ordered overlay and fixed 4 MiB or
32,768-entry chunks. Buffered active compaction keeps the source logical
transaction ID in the dense checkpoint, copies the WAL prefix outside the
publication lock, copies only the concurrent tail inside it, publishes the
byte-identical WAL on a new inode, and then replaces the main file. It never
applies concurrent changes to the replacement B+ tree.

Three fresh alternating pairs used one deterministic 100,000-key fixture. Each
process began from a byte-identical clone. The harness required full reopened
verification, exact agreement with the compaction report, one expected stale
generation conflict, and no lost concurrent commit.

| Median | Baseline | WAL handoff | Change | Gate |
| --- | ---: | ---: | ---: | ---: |
| Maximum RSS | 25,608,192 B | 10,108,928 B | -60.52% | at most -25% |
| Compaction wall time | 3.886 s | 1.966 s | -49.41% | at most +15% |
| Publication-lock pause | 30.108 ms | 26.514 ms | -11.94% | at most +25% |
| Concurrent commit p99 | 18.745 ms | 11.280 ms | -39.82% | at most +25% |
| Final main plus WAL | 12,218,368 B | 12,206,008 B | -0.10% | at least 5% reduction from source |

The candidate reduced its 337,825,988-byte source by 96.39%. Every run reported
one expected conflict and preserved the complete current logical snapshot. A
successful concurrent run also compared the WAL before and after publication
byte-for-byte, while a no-concurrent-suffix run exercised the existing safe
covered-WAL retirement path.

The separate 1,000-operation W6 control produced the same 100,000 entries,
logical SHA-256
`41acebe28eed582abf70c8778dd9182e9c9be1d44c04538ce60737c5ed9f494c`,
main-file SHA-256
`c3a5e737de2892eac11dfe3e5254cd5d522ef562730ecee71dcd4910bf08c1f3`,
and WAL SHA-256
`23b3aeb246f627dff6215418731bd179d935019e73f4138c36488902a458637c`
for baseline and candidate. Formatting, the complete all-target all-feature
suite, focused crash/I/O suites, all-target check, Clippy with warnings denied,
and `git diff --check` passed.

Raw binaries, hashes, resource measurements, and stdout/stderr are retained
under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-wal-handoff-compaction`.

This accepts bounded-memory buffered maintenance and WAL-handoff publication.
It does not close filesystem power-loss testing, automatic scheduling, the
pinned RocksDB matrix, or the sequential W6 mutation-tail gate.
