# Current 1M RocksDB comparison

Date: 2026-10-03. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is a diagnostic for one dataset and platform, not a stable-release claim.

- WSLDB W1/W3/W4 commit: `378fe993ac721bdeaea457b693428d613c7fa557`
- WSLDB W2 commit: `63aa17bc2121f9139438b733f7b39538fff6db75`
- WSLDB W7 commit: `47016b4873a7bc590a41e1a208ba2eeb54f79ef0`
- WSLDB worktree: clean
- RocksDB tag: `v11.8.1`
- RocksDB commit: `abeebd9630f11bd08c28b7bd43c7bdfc62050654`
- RocksDB source archive SHA-256:
  `618d9726a7cb1cf4ce034f4cdca49de98aa64867dda06b91371a791ae8921aff`
- RocksDB Homebrew bottle SHA-256:
  `ef95ef8a35623e1dd9a8302ef767c12f3638d56e6ec56b099ab70692fe7bdbca`
- Locally linked official `db_bench` executable SHA-256:
  `c688b94daa51b454adf782ef114ae28291fd608481c77a86387425ac363272b1`
- W2 `db_bench` executable SHA-256:
  `432e957241f87bff4d31a0dbf2df8609c6ecb314bbef8cbf9cb632286146c7f1`
- W2 WSLDB benchmark executable SHA-256:
  `da22ddcc494e12405d0d7aec4ac27f335f5617e42190b2037905562a0b100182`
- W7 `db_bench` setup executable SHA-256:
  `cbee64c8aabb1e5c1e2d7d0ab181724b79e044ed84e171d2a9e1994a68e19903`
- Exact W7 RocksDB adapter SHA-256:
  `6d5be7faf8ba3e6e8dcb08736f65a22f81c5fd1c392c1c5b364cd2e3ba3546ac`
- Dataset: 1,000,000 entries, 16-byte keys, 100-byte values
- Key encoding: byte-identical RocksDB `db_bench` `GenerateKeyFromInt` output
- Cache budget: 256 MiB for both engines
- Load batch: 10,000 operations
- Durability: synchronous WSLDB commits; RocksDB sync and WAL enabled
- Compression: disabled for both engines
- Repetitions: one warm-up followed by five measured runs

The official `db_bench` sources from the release archive were compiled against
the matching RocksDB 11.8.1 arm64 Homebrew library. The extracted library's
Mach-O install names were relocated to the host's Homebrew dependency paths and
the library was ad-hoc signed; no engine code or benchmark source was changed.
`db_bench --version` reported `11.8.1`, and `git ls-remote` resolved the release
tag to the commit recorded above.

The repository comparison runner alternated engine order and retained every
command, stdout, stderr, process-resource report, size report, and host manifest
under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-current-head-native-1m`.
A smaller end-to-end conformance smoke is retained alongside it under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-current-head-native-smoke`.
The exact W7-only run is retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-current-head-native-w7-1m`.
The current exact W2 run, two preceding candidates, and same-host baseline are
retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-hash-point-index-w2-1m`,
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-streaming-verification-w2-1m`,
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-streaming-w2-buffered-1m`,
and
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-baseline-w2-buffered-1m-rerun`.

## W1 sequential durable load

Whole-process wall time includes setup, explicit RocksDB flush and compaction
waiting, WSLDB reporting, and process teardown. It is used here because
RocksDB's `fillseq` interval excludes later flush work.

| Measured run | WSLDB seconds | RocksDB seconds |
| ---: | ---: | ---: |
| 1 | 4.82 | 6.35 |
| 2 | 4.85 | 6.25 |
| 3 | 4.95 | 6.34 |
| 4 | 4.89 | 6.34 |
| 5 | 4.91 | 6.22 |
| Median | 4.89 | 6.34 |

For the same one million durable operations, WSLDB's median whole-process
throughput was 204,499 operations/s versus RocksDB's 157,729 operations/s, a
1.30-times result. WSLDB's in-engine interval is not compared with RocksDB's
`fillseq` interval because the latter omits deferred flush work.

WSLDB used 12,976,128 bytes median maximum RSS versus RocksDB's 132,055,040
bytes. WSLDB therefore used 90.17% less peak memory on W1, while RocksDB used
10.18 times as much. The WSLDB file was 122,552,320 bytes; the median RocksDB
file total was 121,944,313 bytes. WSLDB's logical storage size was 0.50% larger.
Allocated APFS blocks and every per-run value are retained in the raw evidence.

## W2 random durable load

WSLDB used its 64 MiB durable write buffer. The RocksDB interval is
`filluniquerandom`; the WSLDB interval covers the same one million committed
mutations and includes its synchronous threshold checkpoint. Every batch held
10,000 mutations.

| Engine | Five measured runs, thousand ops/s | Median thousand ops/s | Median maximum RSS |
| --- | --- | ---: | ---: |
| WSLDB | 203.597, 204.003, 202.461, 202.215, 205.471 | 203.597 | 234,225,664 B |
| RocksDB | 534.438, 549.990, 543.482, 586.112, 559.866 | 549.990 | 114,819,072 B |

RocksDB delivered 2.70-times the median active-interval throughput. WSLDB used
2.04 times the median process peak RSS. The RSS measurement includes WSLDB's
streaming post-load full logical verification and RocksDB's explicit flush and
compaction, so it is a whole-process result rather than an active-interval
allocation profile.

Median whole-process time was 6.21 seconds for WSLDB and 7.13 seconds for
RocksDB. These totals are not treated as a throughput win: WSLDB finishes with
a recoverable WAL tail and performs full logical verification, while RocksDB
explicitly flushes and waits for compaction after its reported active interval.
WSLDB used 123,449,976 bytes for the main file plus WAL versus RocksDB's
121,951,146-byte median, 1.23% more. WSLDB reported 1.651 database write
amplification and the same final logical hash in every run.

The first candidate streamed the sorted checkpoint directly to final page IDs
and stored version records once in a shared append-only arena. Against clean
commit `1f3beb7` on the same host and settings, median active throughput rose
from 186,763 to 194,588 operations/s, 4.19%; median maximum RSS fell from
671,072,256 to 427,343,872 bytes, 36.32%; and median whole-process time fell
from 7.06 to 6.84 seconds, 3.12%.

The current candidate additionally verifies the base tree and merges buffered
versions in one ordered stream instead of materializing the complete logical
snapshot, and it hashes each base value only once. Relative to the preceding
clean profile, median maximum RSS fell another 47.35%, from 427,343,872 to
224,985,088 bytes, and median whole-process time fell 8.19%, from 6.84 to 6.28
seconds. The active write interval was not changed by this patch; its observed
2.27% rise is treated as run-to-run variation, not as a causal throughput gain.
The file and WAL encoding, persisted bytes, write amplification, and logical
hash remained identical. These are internal improvement results, not evidence
that WSLDB wins W2.

The current candidate replaces the 64 point-index shard maps with hash maps;
the ordered version map still owns range and checkpoint order. Against the
immediately preceding clean profile, the WSLDB active median rose 2.31%, from
199,007 to 203,597 operations/s. The five measured ranges did not overlap.
Median maximum RSS rose 4.11%, from 224,985,088 to 234,225,664 bytes, remaining
inside the preregistered 10% acceptance limit; median whole-process time fell
1.11%, from 6.28 to 6.21 seconds. Persisted bytes and logical hashes were again
identical. RocksDB performance varied between the two profiles, so the engine
comparison uses only paired values from the current run.

## W3 random point reads, 16 readers

Each measured interval performed one million total reads with 50% present and
50% absent keys. The first RocksDB interval in its six-interval process was the
warm-up and is excluded below.

| Engine | Five measured runs, million ops/s | Median million ops/s | Median p99 |
| --- | --- | ---: | ---: |
| WSLDB | 7.871, 6.982, 7.148, 7.585, 7.429 | 7.429 | 4.292 us |
| RocksDB | 2.691, 2.560, 2.642, 2.600, 2.363 | 2.600 | 9.420 us |

WSLDB delivered 2.86-times median throughput and 2.19-times lower median p99.
It used 147,881,984 bytes maximum RSS versus RocksDB's 145,162,240 bytes, a
1.87% memory cost. Whole-process CPU time was 7.53 seconds for WSLDB and 17.19
seconds for RocksDB, 56.2% less for WSLDB.

## W4 ordered ranges of 100 entries

RocksDB completed a separate cache warm-up before its five measured intervals.

| Engine | Five measured runs, thousand ranges/s | Median thousand ranges/s | Median p99 |
| --- | --- | ---: | ---: |
| WSLDB | 126.045, 129.077, 128.969, 129.781, 124.635 | 128.969 | 12.750 us |
| RocksDB | 45.892, 55.636, 60.804, 59.258, 61.473 | 59.258 | 33.840 us |

WSLDB delivered 2.18-times median throughput and 2.65-times lower median p99.
It used 138,166,272 bytes maximum RSS versus RocksDB's 142,573,568 bytes, 3.09%
less memory.

## W7 warm open to first read

The exact adapters time the same interval: database open followed by one point
read of the first byte-identical key. Each sample is a new process. Run zero is
the warm-up and is excluded from the median.

| Measured run | WSLDB milliseconds | RocksDB milliseconds |
| ---: | ---: | ---: |
| 1 | 0.206250 | 27.400750 |
| 2 | 0.200584 | 27.375000 |
| 3 | 0.223750 | 27.516208 |
| 4 | 0.200417 | 27.860416 |
| 5 | 0.213208 | 30.872708 |
| Median | 0.206250 | 27.516208 |

WSLDB reached the first value 133.41 times faster in the median process. Its
median maximum RSS was 1,933,312 bytes versus RocksDB's 9,928,704 bytes, 80.53%
less. This passes the preregistered five-times W7 threshold for this 1M-entry
warm-OS-cache APFS profile. It is not cold-cache or cross-platform evidence.

## Disposition

This current profile shows WSLDB ahead on W1 whole-process durable load and
peak RSS; W3 throughput, p99, and CPU time; and W4 throughput, p99, and peak
RSS. The exact warm W7 interval also clears its preregistered threshold. W2 is
a clear loss in active throughput and peak RSS despite the measured checkpoint
and verification improvements. W3 RSS is 1.87% higher and the WSLDB W1 file is
0.50% larger. The results supersede the earlier same-day 1M profile with
measurements from clean WSLDB commits and the native RocksDB 11.8.1 release
artifacts.

The result does not cover batch sizes 1 and 100, 4096-byte values, 10M or
out-of-RAM datasets, cold-cache operation, the full W5/W6 gate, Linux, or
filesystem power-loss behavior. These omissions and the measured W2 loss
prevent a general claim that WSLDB is faster than RocksDB.
