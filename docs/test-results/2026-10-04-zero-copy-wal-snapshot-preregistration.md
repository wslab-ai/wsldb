# Streaming zero-copy WAL snapshot handoff

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation

The accepted freeze-and-swap checkpoint beats RocksDB 11.8.1 on active
throughput and maximum durable-batch latency for the frozen W2 batch-10,000
profile, but its 314,867,712-byte median peak RSS is 2.69 times RocksDB's.
Checkpoint page construction already borrows the frozen generation. The
remaining avoidable peak occurs while publishing the suffix WAL snapshot:
the live suffix is copied into a complete `ChangeSet`, encoded into another
complete byte vector, and then copied into a newly replayed write buffer.

## Candidate

The WAL replacement will calculate lengths and CRC32C in one borrowed pass and
write the header, ordered entries, and footer through a bounded staging buffer
in a second borrowed pass. It must produce exactly the existing snapshot bytes
without allocating a record-sized vector.

After atomic replacement, the new buffer generation will clone only the
ordered version map nodes. Keys remain shared through their existing `Arc`s,
and the append-only version/value arena is shared across generations. Future
versions append to that arena; an old map still points to its old latest
versions, while a new map can advance independently. An already initialized
point index may be shared because visibility is selected by transaction ID.
No persistent field, recovery rule, transaction ID, or snapshot-visible value
changes.

The implementation must keep fault injection on every physical snapshot write
and retain metadata-before-WAL publication. It must fail closed if either pass
observes inconsistent entry counts, lengths, ordering, or checksums.

## Frozen acceptance gate

The causal A/B uses one million unique 16-byte keys, incompressible 100-byte
values, 10,000 mutations per synchronous commit, a 64 MiB durable write
buffer, and 65,536 decoded cache pages. The baseline is the storage code at
`9405a78`; one warm-up and five measured fresh-file processes per variant are
alternated, and every process explicitly drains pending work.

The candidate is accepted only if median maximum RSS decreases by at least
30%, median active throughput does not regress by more than 2%, and median
active-plus-drain time does not regress by more than 3%. Median maximum commit
latency may not exceed 25 milliseconds. Final logical, main-file, and WAL
hashes and lengths must match the baseline exactly, and checkpoint failures
must remain zero.

Focused tests must prove byte equality between buffered and streaming snapshot
encoding for puts, tombstones, boundary sizes, and repeated keys; old readers
and an initialized point index must survive handoff and later commits; every
short-write, sync, rename, directory-sync, and process-crash boundary must
recover one complete snapshot. Formatting, all-target all-feature tests,
Clippy with warnings denied, check, diff-check, and the 700-line source limit
must pass. Thresholds will not change after candidate results are observed.

## Result: accepted

The candidate at `f2a792c` passed every frozen acceptance condition. The
baseline was `9405a78`. One warm-up per variant preceded five measured
fresh-file processes per variant, and the measured variants alternated. Both
binaries were built with the release profile from their respective sources.

| Median metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Maximum RSS | 316,211,200 B | 203,669,504 B | -35.59% |
| Active W2 throughput | 635,713.629 ops/s | 642,990.782 ops/s | +1.15% |
| Active load time | 1.573 s | 1.555 s | -1.13% |
| Active plus drain time | 1.896 s | 1.752 s | -7.60% |
| Standalone drain | 312.262 ms | 192.142 ms | -38.47% |
| Maximum commit latency | 19.591 ms | 17.847 ms | -8.91% |
| Maximum sync latency | 39.067 ms | 31.266 ms | -19.97% |
| Cold reopen to first read | 322.061 ms | 323.409 ms | +0.42% |

The primary requirement was at least a 30% RSS reduction; the candidate
achieved 35.59%. Throughput improved rather than consuming its 2% allowance,
active-plus-drain time improved rather than consuming its 3% allowance, and
maximum commit latency remained below the 25-millisecond ceiling. Every run
reported zero checkpoint failures.

All ten measured processes produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`,
main-file hash
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`,
and WAL hash
`21ba0081c157f7ca7a34adc93fd26e72c8ea3623abff1261d0be43cc9081eedd`.
Main-file and WAL lengths were respectively 65,486,848 and 57,960,068 bytes
in every run. Separate cold processes reopened and verified every retained
database.

The implementation preserves the existing snapshot encoding byte for byte.
It calculates CRC32C incrementally, emits the record through a bounded 256 KiB
staging buffer, shares immutable keys and the append-only version arena across
the handoff, and shares an initialized point index while transaction IDs keep
old readers isolated. Focused tests cover encoding equality across a staging
boundary, tombstones, repeated keys, readers spanning the handoff, later
commits, partial writes in each staged region, sync and rename failures, crash
boundaries, corruption, and reopen.

The baseline and candidate benchmark SHA-256 values are respectively
`e5ff5ccf1d37740db5ceb63801968291333b89ffd62990e81eb4da9193a5a5cb`
and
`49fbecf164e71f3fd37bda11c6698d48c00e6ad4ad76ed03e44959398c03d7dc`.
Raw standard output, diagnostics, resource reports, hashes, sizes, cold-open
reports, binaries, and retained databases are outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-zero-copy-wal-snapshot-w2-ab`.
