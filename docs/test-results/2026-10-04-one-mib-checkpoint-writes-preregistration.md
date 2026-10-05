# One-MiB checkpoint page writes

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

The empty-tree checkpoint path emits already ordered immutable 4 KiB pages
through a 256 KiB staging buffer. The current W2 comparison is ahead of
RocksDB in active throughput, but its median maximum durable-batch latency is
297.086 milliseconds versus 46.309 milliseconds for RocksDB. Checkpoint work
remains the dominant WSLDB tail event.

The candidate changes only the production staging capacity from 256 KiB to
1 MiB. It should reduce positioned-write calls by approximately four times
without changing page IDs, encoded bytes, checksums, metadata, WAL contents,
or ordering. Crash-injection builds continue to flush at every existing
per-page boundary, so their observation granularity remains unchanged.

## Frozen acceptance gate

The causal A/B uses one million unique 16-byte keys, 100-byte values, 10,000
mutations per synchronous commit, a 64 MiB durable write buffer, and 65,536
decoded cache pages. It runs one warm-up and five measured fresh-file
processes per variant, alternating measured variants.

The candidate is accepted only if median maximum commit latency decreases by
at least 5%, median active W2 throughput does not regress by more than 1%,
median maximum RSS grows by no more than 1%, and every main-file, WAL, and
logical hash and length is byte-identical. The complete formatting,
all-target all-feature test, Clippy, check, and diff-check gate must pass.
Thresholds will not change after candidate results are observed.

## Result: rejected

The candidate was removed from the source tree. One warm-up and five measured
fresh-file processes were run per variant, alternating the measured variants.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Median active throughput | 546,553.765 ops/s | 539,068.907 ops/s | -1.37% |
| Median maximum commit latency | 243.153 ms | 243.229 ms | +0.03% |
| Median maximum sync latency | 36.531 ms | 37.130 ms | +1.64% |
| Median maximum RSS | 127,631,360 B | 127,680,512 B | +0.04% |

The maximum-commit-latency gate required at least a 5% reduction; the result
was neutral. The throughput guard allowed at most a 1% regression, but the
candidate regressed by 1.37%. All measured logical hashes matched the accepted
baseline. Reported main-file length, total storage length, and write-accounting
bytes were also unchanged. The persisted-file hash gate was not needed after
the two primary performance conditions had already failed.

The experiment shows that the accepted 256 KiB staging capacity has already
removed positioned-write call count as the dominant remaining tail cost.
Reducing the checkpoint pause now requires moving or partitioning construction
and synchronization work rather than increasing this buffer again. Raw output
and both binaries are retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-one-mib-checkpoint-writes-w2-ab`.
