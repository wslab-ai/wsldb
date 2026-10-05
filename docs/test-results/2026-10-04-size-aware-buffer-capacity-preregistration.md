# Size-aware write-buffer capacity

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

The packed value arena appends approximately one megabyte per W2 commit. Its
geometric `Vec` growth gives 67,108,864 bytes of capacity to 54,000,000 live
value bytes immediately before the threshold checkpoint, and the final
46,000,000-byte tail retains the same capacity. Reallocation can also make the
old and new regions resident at once. WAL payload construction independently
grows another vector despite the canonical change set already containing all
sizes needed to plan it.

The candidate will maintain checked logical-value and encoded-payload byte
counts as part of the canonical change set. WAL encoding will allocate the
exact known payload size. Before publishing a durable record, the packed value
arena will grow in bounded 8 MiB increments based on the record's cached value
bytes, avoiding both per-value allocation and unconstrained doubling.

Mutation replacement semantics, canonical key order, maximum input limits,
WAL bytes, transaction IDs, checkpoint thresholds, snapshots, and every
persistent-format field remain unchanged.

## Frozen acceptance gate

The causal A/B uses one-million unique 16-byte keys, 100-byte values, 10,000
mutations per synchronous commit, a 64 MiB durable write buffer, and 65,536
decoded cache pages. It runs one warm-up and five measured fresh-file processes
per variant, alternating measured variants.

This memory-focused candidate is accepted only if median maximum RSS decreases
by at least 5%, median active W2 throughput does not regress by more than 2%,
every main-file, WAL, and logical hash matches, repeated-mutation byte
accounting and arena-capacity tests pass, all snapshot/history, checkpoint,
crash, corruption, and recovery tests pass, and the complete formatting,
all-target all-feature test, Clippy, check, and diff-check gate passes. Commit
and sync latency are reported. Thresholds will not be changed after results are
observed.

## Result

Rejected. Median maximum RSS decreased from 159,318,016 to 158,318,592 bytes,
only 0.63%, and therefore missed the frozen 5% memory threshold. Median active
W2 throughput moved from 472,516.994 to 472,863.702 operations/s, a neutral
0.07% increase. Median total commit time decreased by 0.35%, median maximum
commit latency increased by 1.52%, median recorded `sync_total_ns` decreased by
1.52%, and median maximum sync latency increased by 2.57%.

All warm-up and measured runs produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
All main files had hash
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`
and length 65,486,848 bytes; every WAL had hash
`112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68`
and length 57,963,128 bytes. Raw evidence is retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-size-aware-buffer-capacity-w2-ab`.

The unused geometric capacity was not materially resident on this host, so
constraining it did not solve the measured memory gap. Cached size fields,
bounded arena growth, exact WAL allocation, and their tests were removed. The
production implementation remains the accepted packed-arena baseline.
