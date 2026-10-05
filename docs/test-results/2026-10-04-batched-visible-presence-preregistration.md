# Batched write-buffer presence lookup

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

The W2 commit path currently acquires and releases the version-arena read lock
once per mutation while determining whether each key exists in the durable
write buffer. The transaction already holds the immutable write-buffer state
guard, and all mutations are traversed in canonical key order.

The candidate will validate the snapshot once and hold one version-arena read
guard while collecting the presence of every mutation in a batch. Entry-count
calculation, checkpoint lookups, point-index validation, WAL bytes, publication
order, and locking around the durable append remain unchanged. The explicit
group-commit path will use the same batched lookup independently for each
ordered transaction.

## Frozen acceptance gate

The causal A/B uses one-million 16-byte keys, 100-byte values, 10,000 mutations
per synchronous commit, a 64 MiB durable write buffer, and 65,536 decoded cache
pages. It runs one warm-up and five measured fresh-file processes per variant,
alternating measured variants.

The candidate is accepted only if median active W2 throughput improves by at
least 3%, median maximum RSS grows by no more than 2%, every main-file, WAL, and
logical hash matches, focused snapshot/presence tests pass, and the complete
formatting, all-target all-feature test, Clippy, check, and diff-check gate
passes. Maximum commit and sync latency are reported. Thresholds will not be
changed after results are observed.

## Result

Rejected. The preregistered alternating A/B produced median active throughput
of 450,285.498 operations/s for the baseline and 452,812.666 operations/s for
the candidate, only a 0.56% improvement against the frozen 3% threshold.
Median maximum commit latency decreased from 344,071,666 to 333,990,916
nanoseconds, while median maximum RSS increased from 176,324,608 to 176,766,976
bytes. These secondary results do not override the throughput gate.

Every warm-up and measured run produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
All measured main files had hash
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948` and
length 65,486,848 bytes; every WAL had hash
`112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68` and
length 57,963,128 bytes. Raw evidence is retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-batched-visible-presence-w2-ab`.

The implementation and its focused test were removed. The production path is
unchanged from the baseline.
