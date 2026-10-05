# Buffered index preflight

Date: 2026-10-03. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before the candidate is implemented or
measured.

## Observation and candidate

A two-second late-W2 sample of commit `d41cee2` attributes 125 flat samples to
`PointIndex::preflight` and 61 to `WriteBuffer::visible_value`, while the
membership-filtered `tree::get` path has only five. The current commit path
looks up every changed key in the ordered write buffer to compute entry count,
then `PointIndex::preflight` repeats the ordered lookup while checking the point
index. The first lookup also clones a visible value even though commit needs
only presence.

The candidate will return presence without cloning values and retain that
ordered-index membership result for point-index preflight. Preflight must still
check every affected point-index key and reject a disagreement before the WAL
append. It may not change lock ordering, durability, the public API, persistent
bytes, or the database format.

## Frozen acceptance gate

The causal A/B uses the one-million-entry W2 configuration: 16-byte keys,
100-byte values, 10,000 mutations per synchronous commit, a 64 MiB durable
write buffer, and 65,536 decoded cache pages. It runs one warm-up and five
measured fresh-file processes per variant, alternating measured variants.

The candidate is accepted only if:

- median active W2 throughput improves by at least 3%;
- median maximum RSS grows by no more than 2%;
- every warm-up and measured main file, WAL, and logical result matches;
- disagreement and visibility tests prove the retained fail-closed behavior;
- the full formatting, test, Clippy, check, and diff-check gate passes.

The thresholds and workload parameters are frozen before candidate results are
observed and will not be relaxed afterward.

## Causal A/B result

The candidate passed every frozen threshold. One warm-up and five measured
fresh-file processes were run per variant, with measured variants alternated.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Median active throughput | 347,900 ops/s | 381,591 ops/s | +9.68% |
| Median commit time | 2.505 s | 2.255 s | -9.96% |
| Median sync time | 0.921 s | 0.924 s | +0.33% |
| Median maximum RSS | 176,406,528 B | 177,143,808 B | +0.42% |

Candidate throughput ranged from 353,590 to 386,585 ops/s; baseline throughput
ranged from 344,086 to 351,718 ops/s. Page-cache hits, misses, and resident
pages were identical in every run, isolating the improvement from the preceding
membership-filter change.

All 12 main files and all 12 WALs, including warm-ups, were byte-identical
within their respective file kind. Every process produced the same one-million
entry logical hash. Raw manifests, binaries and hashes, process output,
resource reports, persisted-file hashes, sorted samples, and summary are under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-buffer-preflight-w2-ab`.

## Current RocksDB diagnostic

A fresh alternating run compared the candidate with the pinned official
RocksDB 11.8.1 `db_bench`. Both engines used synchronous WAL-backed commits,
one-million 16-byte keys, 100-byte values, 10,000-mutation batches, one warm-up,
and five measured processes.

| Metric | WSLDB | RocksDB 11.8.1 |
| --- | ---: | ---: |
| Median active throughput | 357,281 ops/s | 516,055 ops/s |
| Median maximum RSS | 176,996,352 B | 114,819,072 B |
| Median whole-process wall time | 3.83 s | 7.25 s |
| Logical storage bytes | 123,449,976 B | 121,951,146 B median |

RocksDB remains 1.44 times faster in the active interval and WSLDB uses 1.54
times its median whole-process peak RSS. WSLDB's shorter whole-process time is
not a throughput win because RocksDB explicitly flushes and waits for
compaction outside its reported load interval. Evidence is retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-buffer-preflight-rocksdb-w2-1m`.

## Correctness verification

The point index still checks every affected key against the ordered membership
result and rejects disagreement before durable append. Its existing explicit
disagreement test passes, as do buffered mixed mutations, checkpoint recovery,
group commits, failure injection, and the complete final-source gate:

- `cargo fmt --all -- --check`
- `cargo test --all-targets --all-features`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo check --all-targets --all-features`
- `git diff --check`
