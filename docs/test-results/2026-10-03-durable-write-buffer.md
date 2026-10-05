# Durable write-buffer diagnostic

Date: 2026-10-03. Platform: Apple/APFS development host. This is a local
diagnostic from the working tree based on commit `3cb62c7`, not a release or
RocksDB comparison.

The candidate adds an opt-in, checksummed write-ahead sidecar. A commit syncs
one canonical transaction record and checkpoints the accumulated mutations
into the copy-on-write page tree after a configured byte threshold. Direct
page-tree commits remain the default.

## W2 random durable load

Configuration: 100,000 deterministic 16-byte keys, 100-byte values, 10,000
operations per durable commit, and 65,536 decoded cache pages. Every sample used
a fresh database. Direct and 64 MiB buffer runs were interleaved after one
warm-up in each mode. The threshold was intentionally larger than this dataset,
so these measurements isolate buffered commits without a checkpoint stall.

| Run | Direct COW ops/s | 64 MiB buffer ops/s |
| ---: | ---: | ---: |
| 1 | 68,542.582 | 330,669.332 |
| 2 | 68,377.810 | 313,537.614 |
| 3 | 69,721.940 | 322,516.575 |
| 4 | 69,081.200 | 315,248.284 |
| 5 | 67,836.764 | 321,042.705 |
| Median | 68,542.582 | 321,042.705 |

Median throughput was 4.68 times direct COW. Process-visible database write
amplification fell from 7.636 to 1.086. Direct COW wrote 88,576,000 database
bytes and ended at 88,543,232 file bytes; the buffered run wrote 12,600,680 WAL
bytes and occupied 12,608,872 bytes including the empty main file.

A separate run used a 4 MiB threshold, forcing two checkpoints and leaving the
last partial buffer outstanding. It reached 106,073.574 ops/s, 1.55 times the
direct median, with 2.569 cumulative database write amplification. This is a
single checkpoint-path diagnostic, not a five-run claim.

## Read-side results and tradeoffs

The final sharded point index uses sorted vectors and batch merges rather than
hash tables. Same-host W3 runs with a 100,000-entry dataset, one million point
operations, and 16 readers measured these throughputs:

| Mode | Five runs, million ops/s | Median million ops/s |
| --- | --- | ---: |
| Direct COW | 5.842, 6.139, 6.918, 6.307, 6.765 | 6.307 |
| Outstanding buffer | 8.544, 7.737, 8.771, 8.259, 7.378 | 8.259 |

The in-memory buffer raised median throughput by 30.9%, while median p99 was
4.084 microseconds versus 3.292 microseconds for direct COW. W4 range-100
medians were 202,692 buffered and 157,472 direct ranges/s; their median p99s
were 5.375 and 7.791 microseconds respectively. Reopening and reconstructing an
outstanding 12.6 MB buffer took about 174.1 ms in repeated standalone W7
diagnostics, compared with sub-millisecond clean direct opens.

Buffering remains disabled by default: the favorable resident-data results
consume extra memory and shift work into recovery and checkpoint stalls. It is
useful only when those costs fit the workload. A production choice also needs
sustained checkpoint-tail measurements at the application's actual dataset and
device size.

## Shared key and version blocks

A follow-up A/B diagnostic compared commit `9bda927` with a candidate
that shares immutable key and MVCC-version allocations between the ordered
buffer and its point index. On the same W2 configuration, the baseline runs
were 333,036.792, 328,477.658, 321,196.436, 321,733.802, and 320,167.555
ops/s. Candidate runs were 342,609.973, 345,904.711, 336,635.351, 338,487.941,
and 344,418.540 ops/s. Medians improved from 321,733.802 to 342,609.973
ops/s, or 6.49%.

Repeated W7 process measurements of the same 12.6 MB outstanding buffer reduced
maximum RSS from 42,172,416 to 41,074,688 bytes, peak memory footprint from
28,442,960 to 27,328,848 bytes, and retired instructions by 2.98%. Comparable
warm open-to-first-read samples were 177.98 ms and 176.92 ms. W3 with one
reader improved 3.57%; interleaved 16-reader runs were scheduler-noisy and had
effectively equal aggregate medians, so no multithreaded gain is claimed for
this allocation change.
