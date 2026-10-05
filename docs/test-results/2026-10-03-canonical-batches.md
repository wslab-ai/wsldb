# Canonical batch application diagnostic

Date: 2026-10-03. Platform: Apple/APFS development host. This is a local
diagnostic against commit `2a2b902`, not a release or RocksDB comparison.

Configuration: 100,000 deterministic 16-byte keys, 100-byte values, 10,000
operations per durable commit, and 65,536 decoded cache pages. Each sample used
a fresh database. Baseline and candidate runs were interleaved, with their
execution order reversed on alternating pairs.

The candidate applies the final mutation for each key in canonical bytewise
order and no longer retains a duplicate first-insertion-order key vector. The
logical result, public API, file encoding, and durability protocol are
unchanged.

## W2 random durable load

| Run | Baseline ops/s | Candidate ops/s |
| ---: | ---: | ---: |
| 1 | 55,499.520 | 63,515.968 |
| 2 | 58,796.327 | 65,714.757 |
| 3 | 58,760.532 | 64,559.978 |
| 4 | 57,360.384 | 67,280.016 |
| 5 | 59,449.786 | 65,565.630 |
| Median | 58,760.532 | 65,565.630 |

Median throughput improved by 11.58%. Database write amplification fell from
7.685 to 7.636. The final file decreased from 89,112,576 to 88,543,232 bytes,
and reachable live bytes decreased from 17,772,544 to 16,863,232.

## W1 sequential durable load

| Run | Baseline ops/s | Candidate ops/s |
| ---: | ---: | ---: |
| 1 | 322,855.680 | 343,129.588 |
| 2 | 356,216.006 | 366,395.488 |
| 3 | 348,643.276 | 365,198.750 |
| 4 | 344,085.627 | 365,612.721 |
| 5 | 345,505.939 | 364,728.227 |
| Median | 345,505.939 | 365,198.750 |

Median throughput improved by 5.70%. Physical write bytes and amplification
remained identical at 11,300,864 bytes and 0.974.
