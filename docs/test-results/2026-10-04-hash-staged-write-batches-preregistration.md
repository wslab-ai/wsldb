# Hash-staged canonical write batches

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

The post-CRC W2 profile attributes 52 flat samples to inserting user mutations
into `ChangeSet`'s `BTreeMap`. This orders the batch after every mutation even
though persistence consumes it only after the batch is complete. The installed
RocksDB 11.8.1 headers also confirm that its ordinary `WriteBatch` is backed by
an append-oriented representation; WSLDB cannot copy that representation
because its transaction contract collapses repeated mutations to the final
value before entry-count calculation and WAL encoding.

The candidate will separate mutable transaction staging from the canonical
persistence change set. Public write transactions and group batches will use a
randomized standard hash map for read-your-writes and final-value replacement.
At commit, ownership moves into a compact vector and the vector is sorted once
by raw key bytes. Infrastructure continues to receive only a canonical ordered
change set. WAL ordering, duplicate semantics, entry counts, file bytes, and
durability are unchanged.

## Frozen acceptance gate

The causal A/B uses one-million 16-byte keys, 100-byte values, 10,000 mutations
per synchronous commit, a 64 MiB durable write buffer, and 65,536 decoded cache
pages. It runs one warm-up and five measured fresh-file processes per variant,
alternating measured variants.

The candidate is accepted only if median active W2 throughput improves by at
least 3%, median maximum RSS grows by no more than 2%, every main-file, WAL, and
logical hash matches, repeated-mutation and canonical-order tests pass, and the
complete formatting, all-target all-feature test, Clippy, check, and diff-check
gate passes. Maximum commit and sync latency are reported. Thresholds will not
be changed after results are observed.

## Result

Rejected. Median active W2 throughput increased from 465,717.831 to
471,422.913 operations/s, a 1.23% improvement that does not meet the frozen 3%
gate. Median maximum commit latency decreased from 323,906,583 to 319,272,541
nanoseconds. Median maximum RSS decreased from 177,061,888 to 175,570,944 bytes.

Every warm-up and measured run produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
All measured main files had hash
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948` and
length 65,486,848 bytes; every WAL had hash
`112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68` and
length 57,963,128 bytes. Raw evidence is retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-hash-staged-w2-ab`.

The candidate implementation and its tests were removed. The production path
retains the baseline `BTreeMap` change set.
