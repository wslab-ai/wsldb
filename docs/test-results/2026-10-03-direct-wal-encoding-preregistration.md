# Direct WAL encoding preregistration

Date: 2026-10-03. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before the candidate is implemented or
measured.

## Observation and candidate

Early and middle samples of W2 at commit `95ba570` show `wal::encode_record`
as a leading controllable CPU symbol, together with `memmove`, allocation, and
reallocation work. The current encoder incrementally builds a payload vector,
allocates a second complete-record vector, and copies the payload into it.

The candidate will compute the exact encoded length from the already measured
logical mutation bytes, reserve one final record buffer, encode the payload
directly after its header, then fill the checksummed header and append the
checksummed footer. It may not alter field ordering, checksums, recovery,
durability, public API, or any persistent byte.

## Frozen acceptance gate

The causal A/B uses one-million 16-byte keys, 100-byte values, 10,000 mutations
per synchronous commit, a 64 MiB durable write buffer, and 65,536 decoded cache
pages. It runs one warm-up and five measured fresh-file processes per variant,
alternating measured variants.

The candidate is accepted only if:

- median active W2 throughput improves by at least 3%;
- median maximum RSS grows by no more than 2%;
- every warm-up and measured main file, WAL, and logical result matches;
- WAL round-trip, truncation, corruption, and format-fixture tests pass;
- the full formatting, test, Clippy, check, and diff-check gate passes.

Thresholds and workload parameters are frozen before candidate measurement and
will not be relaxed after results are observed.

## Result: rejected

The candidate preserved exact output and reduced allocation work, but it did
not clear the frozen throughput threshold.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Median active throughput | 375,265 ops/s | 380,628 ops/s | +1.43% |
| Median commit time | 2.298 s | 2.259 s | -1.68% |
| Median sync time | 0.932 s | 0.912 s | -2.19% |
| Median maximum RSS | 175,718,400 B | 174,047,232 B | -0.95% |

All 12 main files, all 12 WALs, and all logical hashes matched across warm-ups
and measured processes. Nevertheless, +1.43% is below the preregistered +3%
acceptance threshold, so the implementation was reverted. It is not part of
the accepted performance total and no RocksDB comparison was run for it.

Raw manifests, binary hashes, process output, persisted-file hashes, sorted
samples, and the summary are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-direct-wal-encoding-w2-ab`.
