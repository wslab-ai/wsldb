# Durable buffer snapshot diagnostic

Date: 2026-10-03. This is a short local diagnostic, not a release claim or a
comparison with another database.

## Question

Does atomically consolidating an outstanding write-ahead transaction chain into
one sorted snapshot record reduce final drain time and storage without changing
the durable commit path?

The baseline is commit `67d0575`. The candidate differs only after an explicit
flush. Five alternating baseline/candidate pairs used 100,000 fixed-size keys,
100-byte values, a 64 MiB write buffer, and five seconds of the uniform W5
workload. Every run started from an independently copied canonical load.

## Results

| Metric, median of five | Baseline | Snapshot candidate | Change |
| --- | ---: | ---: | ---: |
| Final drain | 75.136 ms | 22.184 ms | 3.39 times faster |
| Main file plus WAL | 15,015,936 B | 12,341,672 B | 17.8% lower |
| Allocated main plus WAL | 15,036 KiB | 12,056 KiB | 19.8% lower |
| Active operations/s | 716.585 | 704.692 | 1.7% lower |

The short active-throughput result is inconclusive and is not treated as an
improvement. Timed runs execute different operation counts, so their final
logical hashes are not expected to match each other. Exact equivalence is
instead checked by integration tests that compare the logical hash before and
after flush and after reopen. Crash-injection tests terminate after snapshot
sync, after rename, and after directory sync, then compare every recovered key
and value with the preregistered model.

Raw outputs and the aggregate table are retained outside the repository at
`2026-10-03-wal-snapshot-ab` in the local benchmark-evidence directory.
