# Point-index generation fence

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

The post-CRC W2 profile attributes a material part of active commit CPU to a
second hash-table lookup for every mutation. This preflight currently compares
the affected point-index keys with the ordered write-buffer index before the
durable WAL append. It detects disagreement only among keys in the next commit.

The candidate will give the point index an atomic transaction-generation
fence. Construction publishes the complete reconstructed generation. Durable
application publishes a new generation only after every point update has been
installed. Before any WAL append, the writer must verify that the point-index
generation equals the immutable ordered-state generation. A panic interrupted
between ordered-state and point-index publication therefore leaves a generation
mismatch and the next writer fails closed before durability begins.

The change removes the per-mutation preflight lookup and its temporary sharded
vectors. Point values, version chains, WAL bytes, transaction ordering,
durability syncs, and reader publication order remain unchanged.

## Frozen acceptance gate

The causal A/B uses one-million 16-byte keys, 100-byte values, 10,000 mutations
per synchronous commit, a 64 MiB durable write buffer, and 65,536 decoded cache
pages. It runs one warm-up and five measured fresh-file processes per variant,
alternating measured variants.

The candidate is accepted only if median active W2 throughput improves by at
least 3%, median maximum RSS grows by no more than 2%, every main-file, WAL, and
logical hash matches, focused generation-mismatch and publication tests pass,
and the complete formatting, all-target all-feature test, Clippy, check, and
diff-check gate passes. Maximum commit and sync latency are reported. The
thresholds will not be changed after results are observed.

## Result

Rejected. Median active W2 throughput increased from 472,566.398 to
481,359.957 operations/s, a 1.86% improvement that does not meet the frozen 3%
gate. Median maximum commit latency decreased from 326,347,125 to 320,495,625
nanoseconds. Median maximum RSS decreased from 176,242,688 to 175,931,392 bytes.
Median recorded `sync_total_ns` was effectively unchanged at 840,107,548 versus
840,395,455 nanoseconds.

Every warm-up and measured run produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
All measured main files had hash
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948` and
length 65,486,848 bytes; every WAL had hash
`112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68` and
length 57,963,128 bytes. Raw evidence is retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-point-index-generation-w2-ab`.

The candidate implementation and its tests were removed. The production path
retains the per-key point-index preflight from the baseline.
