# Slicing-by-eight CRC32C

Date: 2026-10-03. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is a local optimization A/B and one pinned RocksDB diagnostic, not a
cross-platform release claim.

## Change and gate

The preceding table-driven CRC32C implementation still consumed a material
part of WAL encoding and checkpoint page encoding in a two-second W2 process
sample. The candidate builds eight 256-entry tables at compile time and folds
eight input bytes per loop iteration. It uses safe Rust, adds no dependency,
and does not change the Castagnoli polynomial, checksum definition, public API,
page encoding, WAL encoding, or database format.

The preregistered W2 acceptance gate required at least 5% higher median active
throughput, no more than 2% higher median maximum RSS, and byte-identical main
files and WALs. Results were not used to change these thresholds.

## Method

- Baseline: `24a41b3`, the accepted one-byte table implementation.
- Candidate: the isolated `src/infrastructure/checksum.rs` change.
- Dataset: 1,000,000 entries, 16-byte keys, 100-byte values.
- Batch: 10,000 mutations per synchronous commit.
- Durable write buffer: 64 MiB.
- Cache: 65,536 decoded pages.
- Workloads: W1 sequential durable load and W2 random durable load.
- One warm-up followed by five measured fresh-file processes per variant and
  workload, with measured variants alternated.

Raw manifests, executable hashes, stdout, stderr, resource reports, sorted
samples, summaries, and persisted-file hashes are retained under:

- `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-slicing-by-8-w1-ab`
- `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-slicing-by-8-w2-ab`
- `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-slicing-by-8-rocksdb-w2-1m`

## Causal A/B results

| Workload and metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| W1 median throughput | 319,564 ops/s | 414,723 ops/s | +29.78% |
| W1 median commit time | 2.798 s | 2.084 s | -25.55% |
| W1 median sync time | 1.391 s | 0.881 s | -36.62% |
| W1 median maximum RSS | 171,622,400 B | 171,229,184 B | -0.23% |
| W2 median throughput | 248,466 ops/s | 320,423 ops/s | +28.96% |
| W2 median commit time | 3.656 s | 2.758 s | -24.56% |
| W2 median sync time | 1.443 s | 0.918 s | -36.37% |
| W2 median maximum RSS | 233,832,448 B | 233,816,064 B | -0.01% |

All five candidate W1 samples exceeded every baseline W1 sample. All five
candidate W2 samples exceeded every baseline W2 sample. The candidate passed
the preregistered W2 gate.

Every baseline, candidate, and recorded W2 warm-up produced the same main-file
SHA-256 and the same WAL SHA-256. W1 main-file and WAL hashes were likewise
identical across all ten measured processes. The logical hash, storage size,
and 1.651 database write amplification also remained unchanged. This
establishes byte-for-byte format compatibility for the measured datasets.

These percentages are not added to the preceding table-driven CRC32C result:
the two causal experiments used separate baseline samples.

## Current RocksDB W2 diagnostic

The repository runner alternated the candidate with the pinned official
RocksDB 11.8.1 `db_bench` executable. Both engines used synchronous WAL-backed
commits, 100-byte values, 10,000-mutation batches, one warm-up, and five
measured processes.

| Metric | WSLDB | RocksDB 11.8.1 |
| --- | ---: | ---: |
| Median active throughput | 313,631 ops/s | 508,915 ops/s |
| Median maximum RSS | 233,013,248 B | 114,819,072 B |
| Median whole-process wall time | 4.15 s | 7.27 s |
| Logical storage bytes | 123,449,976 B | 121,951,146 B median |

RocksDB remains 1.62 times faster in the measured active interval and WSLDB
uses 2.03 times its median whole-process peak RSS. WSLDB's shorter whole-process
time is not treated as a throughput win: its final state includes a recoverable
WAL tail, while the RocksDB runner explicitly flushes and waits for compaction.

## Correctness verification

The implementation matches the standard `123456789` CRC32C check value and the
previous bitwise implementation for every prefix of a deterministic 512-byte
corpus. A second deterministic test covers 32,800 combinations of varied
content, length, and starting offset. The following final-source checks passed:

- `cargo fmt --all -- --check`
- `cargo test --all-targets --all-features`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo check --all-targets --all-features`
- `git diff --check`

The opt-in release corruption matrix also passed 100,000 deterministic
main-file mutations and 100,000 deterministic WAL mutations. Every accepted
input matched a known complete state; all other inputs failed closed. Both
targets passed with zero failures in 4,020.82 seconds. Raw output and the
process-resource report are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-slicing-by-8-corruption-100k`.
This finite APFS result is not a physical-power-loss or cross-platform claim.
