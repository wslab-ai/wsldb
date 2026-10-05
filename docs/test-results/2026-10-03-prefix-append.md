# Prefix compression and monotonic append diagnostic

Date: 2026-10-03. Platform: the same Apple/APFS development host used by the
2026-10-02 comparison. This is a local diagnostic, not stable-release evidence.

Configuration: 1,000,000 ascending 16-byte keys, deterministic 100-byte values,
10,000 operations per durable commit, 65,536 decoded cache pages. Every commit
uses the normal data and metadata sync protocol.

## Leaf common-prefix encoding

Five fresh-file runs after leaf prefix compression:

| Run | Operations/s | Final bytes |
| ---: | ---: | ---: |
| 1 | 76,269 | 113,586,176 |
| 2 | 75,165 | 113,586,176 |
| 3 | 74,414 | 113,586,176 |
| 4 | 74,926 | 113,586,176 |
| 5 | 74,709 | 113,586,176 |

The median was 74,926 operations/s. Relative to the preceding five-run median
of 76,808 operations/s and 127,197,184 bytes, throughput changed by -2.45% and
file size by -10.70%. W3/t16 measured-run median was 4.64 million operations/s;
W4 range-100 median was 136,861 ranges/s.

## Batched monotonic tree construction

Five fresh-file runs after adding the right-spine batch path:

| Run | Operations/s | Final bytes |
| ---: | ---: | ---: |
| 1 | 309,046 | 112,693,248 |
| 2 | 328,772 | 112,693,248 |
| 3 | 314,818 | 112,693,248 |
| 4 | 295,625 | 112,693,248 |
| 5 | 293,798 | 112,693,248 |

The median was 309,046 operations/s when the five values are ordered, 4.12x the
prefix-only median. W3/t16 measured-run median was 4.78 million operations/s;
W4 range-100 median was 141,358 ranges/s. The earlier official RocksDB W1
median was 157,729 operations/s, but the complete comparison runner has not yet
been repeated on this commit, so the cross-engine ratio remains provisional.

One batch-size-100 diagnostic completed at 8,112 operations/s and 238,256,128
bytes. Its 10,000 durable syncs dominate elapsed time, while immutable pages
retired by those commits account for most of the extra file size until
compaction.

The append-specific recovery test interrupts every page-write position and all
three publication boundaries. After each interruption, uncached verification
accepts only the previous snapshot or the complete appended snapshot.
