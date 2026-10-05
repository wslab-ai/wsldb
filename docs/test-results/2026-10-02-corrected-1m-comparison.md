# Corrected 1M RocksDB comparison — 2026-10-02

## Scope

- WSLDB commits: `9ad70389848be4d2e1f9705e5f84661f3d5386ef` for the
  complete W4 matrix and `55990c05476a9162e04e3ffe21482f7368830828` for the
  equal-operation W1/W3/W4-10 confirmation; exact W7 was confirmed on
  `c57169726582eb3410ab4e69293929275d7dcdfe`
- RocksDB commit: `abeebd9630f11bd08c28b7bd43c7bdfc62050654`
  (release 11.8.1)
- Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2
- Dataset: 1,000,000 entries, 16-byte keys, 100-byte values
- Cache budget: 256 MiB for both engines
- Durability: synchronous WSLDB commits; RocksDB sync and WAL enabled; no
  compression; batch size 10,000
- Repetitions: one warm-up followed by five measured runs

The runner used the same total operation count for threaded reads. RocksDB's
`--reads` value is per thread, so the t16 adapter passed 62,500 reads per thread
and verified that every repeat reported exactly 1,000,000 operations. W4 used a
separate warm-up large enough to touch an estimated five cache capacities,
capped at one million ranges, for both engines.

Raw command logs, manifests, process-resource reports, and stdout/stderr are
retained outside the repository. The repository runner reproduces their command
construction. This is diagnostic evidence for one dataset and one platform,
not a stable-release result.

## W1 sequential durable load

Whole-process wall time includes RocksDB flush and compaction waiting.

| Measured run | WSLDB seconds | RocksDB seconds |
| ---: | ---: | ---: |
| 1 | 13.26 | 6.35 |
| 2 | 12.71 | 6.37 |
| 3 | 13.23 | 6.34 |
| 4 | 13.28 | 6.26 |
| 5 | 12.65 | 6.23 |
| Median throughput | 75,586 ops/s | 157,729 ops/s |

WSLDB reaches `0.48×` RocksDB here, so sequential write throughput remains an
open performance problem. Monotonic leaf packing reduced the final WSLDB file
from the earlier 244,424,704 bytes to 127,197,184 bytes. RocksDB used
121,944,182 bytes, leaving WSLDB at `1.04×` logical size instead of about `2×`.

## W3 random point reads, 16 readers

Each measured run performs one million total reads with 50% present and 50%
absent keys.

| Measured run | WSLDB ops/s | RocksDB ops/s |
| ---: | ---: | ---: |
| 1 | 6,151,789 | 3,343,106 |
| 2 | 5,854,264 | 3,404,232 |
| 3 | 5,571,442 | 3,226,399 |
| 4 | 5,342,733 | 3,087,820 |
| 5 | 2,047,881 | 3,039,005 |
| Median | 5,571,442 | 3,226,399 |

The median ratio is `1.73×`, above the preregistered `1.25×` threshold for this
profile. The fifth WSLDB run is a material scheduler outlier and is intentionally
shown rather than discarded. Across the warm-up plus five runs, WSLDB used 1.68
seconds wall time, 11.57 seconds user+system CPU, and 246,693,888 bytes maximum
RSS. RocksDB used 1.92 seconds wall time, 15.98 seconds CPU, and 145,653,760
bytes maximum RSS. The throughput win therefore carries a `1.69×` peak-RSS cost
in this run.

## W4 ordered range scans

All values are ranges per second. The n100 and n10000 rows come from the full
corrected warm-up matrix on commit `9ad7038`; the later runner change affects
only W3's per-thread operation count.

| Range | Engine | Run 1 | Run 2 | Run 3 | Run 4 | Run 5 | Median |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 10 | WSLDB | 614,016 | 617,038 | 613,473 | 538,288 | 496,098 | 613,473 |
| 10 | RocksDB | 149,651 | 181,709 | 203,454 | 219,789 | 232,688 | 203,454 |
| 100 | WSLDB | 145,525 | 145,134 | 143,531 | 143,390 | 143,903 | 143,903 |
| 100 | RocksDB | 52,763 | 65,176 | 68,954 | 66,752 | 67,832 | 66,752 |
| 10,000 | WSLDB | 1,542 | 1,569 | 1,603 | 1,608 | 1,599 | 1,599 |
| 10,000 | RocksDB | 1,075 | 1,076 | 1,087 | 1,082 | 1,085 | 1,082 |

The median WSLDB/RocksDB ratios are `3.02×`, `2.16×`, and `1.48×`. All three
pass the preregistered `1.25×` range-scan threshold for this diagnostic profile.
For n10000 including warm-up, WSLDB used 38.95 seconds wall time and 199,770,112
bytes maximum RSS. RocksDB used 55.62 seconds across its separate warm-up and
measured processes and at most 143,736,832 bytes RSS.

## W7 exact warm open-to-first-read

The exact RocksDB adapter times the same interval as WSLDB: database open plus
the first successful point read. It uses the same 256 MiB cache configuration
and the first key written by `db_bench`.

| Measured run | WSLDB ns | RocksDB ns |
| ---: | ---: | ---: |
| 1 | 187,833 | 29,496,292 |
| 2 | 189,375 | 28,980,167 |
| 3 | 189,916 | 27,397,958 |
| 4 | 194,667 | 28,351,292 |
| 5 | 194,375 | 28,799,834 |
| Median | 189,916 | 28,799,834 |

WSLDB is `151.6×` faster for this warm-filesystem-cache profile and passes the
preregistered five-times threshold. The same clean run reconfirmed W3/t16 at
`1.60×` and W4/n10 at `2.60×`. Cold-cache startup remains unmeasured and is not
inferred from this result.

## Disposition

This run closes the earlier 1M-key W3/t16, corrected W4, and warm W7 diagnostic
failures. It does not satisfy the release gate by itself: 10M, 4× RAM,
4096-byte values, cold-cache runs, W5/W6, other operating systems, and durability
matrices remain outstanding. W1 and peak RSS also require further work.
