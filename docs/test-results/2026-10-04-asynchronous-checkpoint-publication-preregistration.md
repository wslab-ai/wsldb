# Asynchronous checkpoint publication

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

The accepted W2 implementation reaches a median 410,026 operations/s against
RocksDB 11.8.1 at 522,952 operations/s. Its automatic checkpoint introduces a
roughly 0.55--0.60 second maximum commit pause. The checkpoint currently builds
immutable pages, syncs them, publishes metadata, and clears the WAL on the
foreground commit path even though the triggering transaction is already
durable in the WAL.

The candidate will retain foreground page construction for this experiment but
move checkpoint data sync off the triggering commit. Later commits may continue
appending to the WAL. Once the data sync completes, a serialized commit or
explicit flush publishes the checkpoint and atomically retains the WAL suffix
after the checkpoint boundary. Recovery must accept either the old checkpoint
with the complete WAL or the new checkpoint with the same WAL suffix after a
crash at every publication boundary.

At most one checkpoint may be pending per storage generation. Explicit flush
must drain it. Benchmark output must report drain work separately; background
work may not be omitted from whole-operation accounting. This experiment does
not claim fully asynchronous checkpoint construction.

## Frozen correctness gate

The candidate is rejected unless all of the following hold:

- the existing full test, corruption, fault-injection, crash-injection, and
  multiprocess suites pass;
- new tests cover a checkpoint with commits appended during data sync, explicit
  drain, sync failure, and crashes before and after metadata publication and WAL
  suffix replacement;
- every recovered state is either the complete pre-publication WAL state or the
  checkpoint plus its complete suffix, never a partial logical snapshot;
- old read transactions retain their pinned snapshots;
- source files remain at or below 700 physical lines and no `unsafe` is added.

## Frozen performance gate

The causal A/B uses one-million 16-byte keys, 100-byte values, 10,000 mutations
per synchronous commit, a 64 MiB durable write buffer, and 65,536 decoded cache
pages. It runs one warm-up and five measured fresh-file processes per variant,
alternating measured variants.

The candidate is accepted only if median active W2 throughput improves by at
least 5%, median maximum RSS grows by no more than 5%, every persisted and
logical hash matches, and median total active-plus-drain time does not regress
by more than 3%. Maximum commit latency and drain latency are reported even when
the throughput threshold passes. Thresholds will not be changed after results
are observed.

## Result: rejected

The candidate was removed from the source tree. Five alternating measured
fresh-file processes were completed for the accepted baseline and candidate.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Median active throughput | 338,617 ops/s | 323,423 ops/s | -4.49% |
| Median maximum commit latency | 688.1 ms | 612.3 ms | -11.01% |
| Median maximum RSS without a baseline drain | 175,980,544 B | 393,363,456 B | not comparable |
| Candidate median drain latency | not measured in paired baseline | 752.1 ms | diagnostic only |

The required active-throughput improvement was at least 5%; the candidate
instead regressed by 4.49%. All ten logical hashes matched. The candidate also
wrote a different WAL representation: its final storage length was 123,446,916
bytes versus 123,449,976 bytes for the baseline, so the frozen persisted-byte
identity condition did not pass.

A separate baseline-with-drain diagnostic completed four runs before the host
filesystem ran out of free space on the fifth. Its approximately 401 MB peak
RSS shows that comparing the candidate's whole-process RSS against a baseline
that did not call `flush()` would be invalid; no memory-regression claim is made
from those samples. The four-run diagnostic is not used for acceptance.

The implementation still constructed checkpoint pages on the foreground path,
then overlapped storage synchronization with later WAL syncs, copied the WAL
suffix, and replayed that suffix while the old buffer remained live. This moved
work rather than removing it and added I/O and recovery overhead. A future
candidate must freeze and swap the mutable partition itself, let the worker own
the frozen partition without copying values, and publish without reconstructing
the active tail beside the complete old state.

Raw process output is retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-asynchronous-checkpoint-publication-screen`.
