# Single-pass version-map update

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

Early and middle W2 samples at commit `95ba570` identify
`WriteBuffer::apply_record_unchecked` as the largest controllable CPU symbol
after synchronization. For every mutation it currently performs an ordered-map
lookup for the previous version and then a second ordered-map traversal to
insert the new version.

The candidate will use one `BTreeMap::entry` traversal to read and replace the
version ID. Version-arena ordering, historical chains, point-index publication,
post-sync infallibility, WAL bytes, and every persistent format field remain
unchanged.

## Frozen acceptance gate

The causal A/B uses one-million 16-byte keys, 100-byte values, 10,000 mutations
per synchronous commit, a 64 MiB durable write buffer, and 65,536 decoded cache
pages. It runs one warm-up and five measured fresh-file processes per variant,
alternating measured variants.

The candidate is accepted only if median active W2 throughput improves by at
least 3%, median maximum RSS grows by no more than 2%, every persisted and
logical hash matches, version-history tests pass, and the full formatting,
test, Clippy, check, and diff-check gate passes. Thresholds will not be changed
after results are observed.

## Causal A/B result

The candidate passed every frozen threshold. One warm-up and five measured
fresh-file processes were run per variant, with measured variants alternated.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Median active throughput | 379,471 ops/s | 407,499 ops/s | +7.39% |
| Median commit time | 2.262 s | 2.090 s | -7.60% |
| Median sync time | 0.928 s | 0.912 s | -1.66% |
| Median maximum RSS | 176,963,584 B | 175,980,544 B | -0.56% |

Candidate throughput ranged from 400,033 to 419,748 ops/s and baseline
throughput from 347,650 to 394,685 ops/s. All 12 main files, all 12 WALs, and
all logical hashes matched across warm-ups and measured processes.

Raw manifests, binary hashes, process output, resource reports, persisted-file
hashes, sorted samples, and summary are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-single-pass-version-map-w2-ab`.

## Current RocksDB diagnostic

A fresh alternating run compared the candidate with pinned official RocksDB
11.8.1. Both engines used synchronous WAL-backed commits, one-million 16-byte
keys, 100-byte values, 10,000-mutation batches, one warm-up, and five measured
processes.

| Metric | WSLDB | RocksDB 11.8.1 |
| --- | ---: | ---: |
| Median active throughput | 410,026 ops/s | 522,952 ops/s |
| Median maximum RSS | 176,799,744 B | 114,819,072 B |
| Median whole-process wall time | 3.37 s | 7.25 s |
| Logical storage bytes | 123,449,976 B | 121,951,146 B median |

RocksDB remains 1.28 times faster in the active W2 interval and WSLDB uses 1.54
times its median peak RSS. The shorter WSLDB whole-process time is not treated
as an active-throughput win because RocksDB explicitly flushes and waits for
compaction outside its timed load interval. Evidence is retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-single-pass-version-map-rocksdb-w2-1m`.

## Correctness verification

The historical-version, point-index, merge-scan, mixed-mutation, checkpoint,
recovery, failure-injection, and format tests all pass. The complete
final-source gate passed:

- `cargo fmt --all -- --check`
- `cargo test --all-targets --all-features`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo check --all-targets --all-features`
- `git diff --check`
