# Single ordered write-buffer index

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

Each buffered key currently appears in both the ordered `BTreeMap` used by
range scans and checkpoints and a 64-shard hash point index. The maps share key
bytes through `Arc`, but the hash table still retains another fat pointer,
version ID, control byte, spare capacity, and allocation metadata per slot.
Commits also validate and update both indexes after already consulting the
ordered map. The duplicate hash index was originally accepted for point-read
speed, before the current packed arena and membership filters existed.

The candidate will make the ordered version map the single source of truth.
Buffered point reads will take the buffer's shared read lock and resolve the
same version chain used by scans. Commits and recovery will stop building,
cross-checking, and updating a duplicate point map. Snapshot visibility,
membership-filter behavior, canonical ordering, WAL bytes, checkpoint logic,
and every persistent-format field remain unchanged.

## Frozen acceptance gate

The primary causal A/B uses W2 with one-million unique 16-byte keys, 100-byte
values, 10,000 mutations per synchronous commit, a 64 MiB durable write
buffer, and 65,536 decoded cache pages. It runs one warm-up and five measured
fresh-file processes per variant, alternating measured variants.

The candidate is accepted only if median W2 maximum RSS decreases by at least
8%, median active W2 throughput does not regress, every main-file, WAL, and
logical hash matches, and the complete correctness gate passes. Because the
removed structure served reads, a byte-identical buffered dataset must also be
measured with 64 point-reader threads: median W3 throughput may regress by no
more than 5%, median p99 may regress by no more than 10%, and throughput must
remain at least 1.25 times the pinned RocksDB reference. Warm open-to-first-read
time is reported but has no independent threshold. Commit/sync latency and
whole-process RSS are reported. Thresholds will not be changed after results
are observed.

## Result

Rejected. The W2 result was strong but the mandatory read guard failed. Median
active W2 throughput increased from 469,662.264 to 521,440.390 operations/s,
an 11.02% improvement, and median maximum RSS decreased from 159,252,480 to
127,205,376 bytes, a 20.12% reduction.

Across the 25 measured 64-reader intervals per variant on the same buffered
dataset, median W3 throughput fell from 5,837,109.608 to 1,218,813.978
operations/s, a 79.12% regression. Median p99 rose from 9,125 to 28,250
nanoseconds, a 211.0% regression. The candidate also fell below the pinned
RocksDB 64-reader reference, independently failing the comparative floor.
Median whole-process W3 RSS decreased from 214,466,560 to 183,582,720 bytes,
but memory cannot override the read-performance contract.

All W2 warm-up and measured runs produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
All measured main files had hash
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`
and length 65,486,848 bytes; every WAL had hash
`112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68`
and length 57,963,128 bytes. Raw output is retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-single-overlay-index-w2-w3-ab`.

The single-index implementation was removed. The measurements motivate a
separately preregistered lazy point index rather than weakening the read gate.
