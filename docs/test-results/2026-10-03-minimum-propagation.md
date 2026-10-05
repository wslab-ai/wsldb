# Subtree minimum propagation diagnostic

Date: 2026-10-03. Platform: the same Apple/APFS development host used by the
other 2026-10-03 diagnostics. This is local diagnostic evidence, not a stable
release or RocksDB comparison result.

Configuration: 100,000 entries, deterministic 16-byte keys and 100-byte
values, 10,000 operations per durable commit, and 65,536 decoded cache pages.
Every commit used the normal data and metadata sync protocol. Each result used
a fresh database file.

The baseline recalculated a branch's minimum by descending its left subtree
after each mutation. The candidate reads the root minimum once per transaction
and carries subtree minima through insert, split, delete, redistribution, and
merge results.

## W2 random durable load

| Run | Baseline ops/s | Candidate ops/s |
| ---: | ---: | ---: |
| 1 | 54,970.061 | 62,806.032 |
| 2 | 57,598.026 | 63,981.316 |
| 3 | 56,558.495 | 62,858.775 |
| 4 | 57,093.242 | 63,215.874 |
| 5 | 57,060.560 | 62,502.388 |
| Median | 57,060.560 | 62,858.775 |

The median improved by 10.16%. Deterministic page-cache hits during the load
fell from 42,583 to 17,498, a 58.91% reduction. The candidate produced exactly
the same physical results as the baseline:

- 89,112,576 final file bytes;
- 89,145,344 database write bytes;
- 7.685 database write amplification;
- 17,772,544 live bytes;
- 71,340,032 retired bytes.

This change reduces redundant decoded-page lookups and CPU work. It does not
solve W2 copy-on-write amplification, which remains an open architecture task.

Per-page-kind counters measured the candidate's deterministic 89,104,384 data
page bytes as 88,293,376 leaf bytes and 811,008 branch bytes, with no overflow
pages for this inline-value workload. Leaf rewrites therefore account for
99.09% of W2 data-page writes; branch pages account for only 0.91%. Further W2
work must reduce repeated leaf rewrites rather than optimize branch encoding.

## W1 sequential durable confirmation

The unchanged monotonic append path produced 351,943.313 median operations/s
across five fresh-file runs. Every run produced 11,268,096 final bytes and
0.974 database write amplification. This confirmation is not a before/after
W1 claim; it checks that the general mutation-path change did not alter the
append path's physical result.
