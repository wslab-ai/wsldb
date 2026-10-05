# Base membership filter

Date: 2026-10-03. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This document records the acceptance contract before the candidate is
implemented or measured.

## Observation and hypothesis

A two-second sample of the accepted slicing-by-eight build during the late W2
random durable-load interval no longer contains CRC32C among its sampled
symbols. Exact base-tree membership checks instead include `tree::get`, packed
leaf search, comparisons, and their page-cache work. These checks are needed
to maintain the exact logical entry count, but most W2 keys are absent.

The candidate will attach an in-memory probabilistic membership filter to the
immutable checkpoint represented by a `BufferedWrites` instance. A negative
answer may skip the exact tree lookup. A positive answer must still use the
tree, so false positives affect performance only. The candidate must establish
the no-false-negative invariant when it creates or carries a filter across a
checkpoint. Reopened databases without a reconstructed filter must retain the
existing exact path. No persistent bytes, public API, durability ordering, or
database format may change.

The expected benefit is lower CPU and page-cache work after the first W2
checkpoint. The expected cost is the filter's memory and hashing.

## Frozen acceptance gate

The causal A/B will use the same one-million-entry W2 configuration as the
current RocksDB diagnostic: 16-byte keys, 100-byte values, 10,000 mutations per
synchronous commit, a 64 MiB durable write buffer, and 65,536 decoded cache
pages. It will run one warm-up and five measured fresh-file processes per
variant, alternating variants.

The candidate is accepted only if all of the following hold:

- median active W2 throughput improves by at least 5%;
- median maximum RSS grows by no more than 2%;
- every measured and warm-up main file, WAL, and logical result matches its
  counterpart;
- focused no-false-negative and checkpoint transition tests pass;
- the full formatting, test, Clippy, check, and diff-check gate passes.

Thresholds and workload parameters are frozen before candidate measurements.
Results will not be used to relax them. A candidate that misses any condition
will be reverted rather than described as an optimization.

## Causal A/B result

The candidate passed the frozen gate. One warm-up and five measured fresh-file
processes were run for each variant, with measured variants alternated.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Median active throughput | 311,931 ops/s | 337,955 ops/s | +8.34% |
| Median commit time | 2.837 s | 2.570 s | -9.41% |
| Median sync time | 0.906 s | 0.918 s | +1.39% |
| Median maximum RSS | 234,209,280 B | 176,406,528 B | -24.68% |

The five candidate throughput samples ranged from 320,716 to 352,613 ops/s;
the five baseline samples ranged from 310,361 to 316,473 ops/s. The candidate
reduced deterministic page-cache misses from 15,883 to 3,359 and resident
decoded pages from 15,986 to 3,462. This explains why removing negative tree
lookups also reduced RSS rather than merely adding the filter's bit array.

All 12 main files and all 12 WALs, including warm-ups, were byte-identical
within their respective file kind. Every run produced the same one-million
entry logical hash. The candidate therefore changes neither logical results
nor persistent encoding for the measured workload.

Raw manifests, executable hashes, output, resource reports, persisted-file
hashes, sorted samples, and the generated summary are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-base-membership-filter-w2-ab`.

## Current RocksDB diagnostic

A new alternating run compared the accepted candidate with the pinned official
RocksDB 11.8.1 `db_bench`. Both engines used synchronous WAL-backed commits,
one-million 16-byte keys, 100-byte values, batches of 10,000, one warm-up, and
five measured processes.

| Metric | WSLDB | RocksDB 11.8.1 |
| --- | ---: | ---: |
| Median active throughput | 347,050 ops/s | 533,165 ops/s |
| Median maximum RSS | 177,356,800 B | 114,786,304 B |
| Median whole-process wall time | 3.84 s | 7.16 s |
| Logical storage bytes | 123,449,976 B | 121,951,146 B median |

RocksDB remains 1.54 times faster in the active W2 interval and WSLDB uses 1.55
times its median whole-process peak RSS. WSLDB's shorter whole-process time is
not a throughput win because the RocksDB command explicitly flushes and waits
for compaction after its timed load interval.

The comparison evidence is retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-membership-filter-rocksdb-w2-1m`.

## Correctness verification

Focused tests cover zero false negatives across 10,000 inserted deterministic
keys, a bounded deterministic false-positive sample, and insert/update/delete
transitions across successive automatic checkpoints. Reopen verifies the final
logical state without relying on the in-memory filter. The complete final-source
gate passed:

- `cargo fmt --all -- --check`
- `cargo test --all-targets --all-features`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo check --all-targets --all-features`
- `git diff --check`
