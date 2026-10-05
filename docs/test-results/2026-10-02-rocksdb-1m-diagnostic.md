# RocksDB 1M diagnostic evidence — 2026-10-02

## Subject

- WSLDB commit: `4cacedc8ea5f214bf4b267cc45a358e81e4a73e8`
- RocksDB commit: `abeebd9630f11bd08c28b7bd43c7bdfc62050654`
- RocksDB release: 11.8.1
- Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2
- Dataset: 1,000,000 entries, 16-byte keys, 100-byte values
- Decoded/block cache budget: 256 MiB
- Repetitions: one warm-up followed by five measured runs

The comparison used `benchmarks/run-comparison.sh`. W1 used synchronous commits,
RocksDB WALs, no compression, and batches of 10,000. W3 used 50% present and
50% absent random reads. The raw evidence directory contained the manifest,
exact commands, stdout, stderr, process resource measurements, and sizes.

## Diagnostic result

| Workload | WSLDB median | RocksDB median | WSLDB / RocksDB |
| --- | ---: | ---: | ---: |
| W1 end-to-end load | 71,276 ops/s | 159,490 ops/s | 0.45× |
| W3, 1 reader | 1,271,820 ops/s | 574,044 ops/s | 2.22× |
| W3, 4 readers | 1,190,081 ops/s | 1,966,876 ops/s | 0.61× |
| W3, 16 readers | 360,499 ops/s | 3,464,237 ops/s | 0.10× |
| W3, 64 readers | 361,901 ops/s | 3,650,306 ops/s | 0.10× |
| W4, range 10,000 | 1,614 ranges/s | 1,088 ranges/s | 1.48× |

The W1 row uses whole-process wall time so RocksDB flush and compaction waiting
remain inside the measurement. Median final sizes were 244,424,704 bytes for
WSLDB and approximately 121,944,000 bytes for RocksDB.

## Findings and disposition

The run is diagnostic, not release evidence. It exposed two problems:

1. A single page-cache lock and globally contended access counters prevented
   read throughput from scaling beyond one reader.
2. The W4 warm-up did not reach a stationary cache state for short ranges;
   successive measured runs accelerated substantially. Those short-range
   results are invalid and intentionally omitted from the table.

The cache was subsequently sharded and its counters distributed. A focused
rerun increased median W3 throughput to approximately 1.93 million ops/s at 16
readers and 2.21 million ops/s at 64 readers. Commit
`9f35b3fe694f9a7104cfed50b2fd273a33be21c5` then added a generation-invalidated,
eight-page reader-local cache. Its clean 1M-key confirmation produced:

| Readers | WSLDB median | RocksDB baseline | WSLDB / RocksDB |
| ---: | ---: | ---: | ---: |
| 1 | 1,264,873 ops/s | 574,044 ops/s | 2.20× |
| 4 | 3,314,986 ops/s | 1,966,876 ops/s | 1.69× |
| 16 | 3,716,481 ops/s | 3,464,237 ops/s | 1.07× |
| 64 | 5,257,544 ops/s | 3,650,306 ops/s | 1.44× |

The preregistered 1.25× read gate therefore still fails at 16 readers. A tested
increase from eight to 32 local slots reduced the 16-reader median and was
reverted; individual outliers were not selected as evidence. The W4 harness now
warms an estimated five cache capacities before measurement. The corrected full
matrix must be rerun from a clean commit.
