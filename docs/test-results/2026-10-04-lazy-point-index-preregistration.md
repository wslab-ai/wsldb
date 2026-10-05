# Lazy buffered point index

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

Removing the duplicate hash point index improved W2 throughput by 11.02% and
reduced peak RSS by 20.12%, but regressed 64-reader W3 throughput by 79.12%.
The W2 load never performs a buffered point lookup; it pays to construct,
validate, and update an index that only a later point read needs. Conversely,
the W3 warm-up can construct that index once before measured reads.

The candidate will retain the existing sharded hash index but initialize it
lazily under the ordered buffer's read lock on the first point lookup. Commits
will cross-check and update the hash index only after it has been initialized.
A writer holds the ordered state lock through optional hash-index publication,
so a reader can observe either a complete pre-commit index or a complete
post-commit index, never a mixed generation. Recovery, write-only loads, scans,
verification, and checkpoints will not allocate the duplicate index.

Snapshot visibility, membership filters, canonical ordering, WAL bytes,
checkpoint behavior, and every persistent-format field remain unchanged.

## Frozen acceptance gate

The W2 A/B repeats the one-million-key, 100-byte-value, 10,000-operation commit,
64 MiB durable-buffer, 65,536-page-cache profile with one warm-up and five
alternating fresh-file processes per variant. Median active throughput must
improve by at least 5%, median maximum RSS must decrease by at least 15%, and
all persistent and logical hashes must match.

On one byte-identical buffered dataset, five alternating measured processes per
variant each run the benchmark's warm-up and five measured 64-reader W3
intervals. Candidate median throughput may regress by no more than 5%, median
p99 may regress by no more than 10%, and throughput must remain at least 1.25
times the pinned RocksDB reference. Concurrent first-read initialization and a
commit following initialization require focused tests. Buffered
open-to-first-read time, commit/sync latency, and whole-process RSS are
reported. The complete formatting, all-target all-feature test, Clippy, check,
and diff-check gate must pass. Thresholds will not be changed after results are
observed.

## Result

Accepted. The final-code W2 alternating A/B increased median active throughput
from 476,211.509 to 530,681.459 operations/s, an 11.44% improvement. Median
maximum RSS decreased from 159,465,472 to 128,466,944 bytes, a 19.44%
reduction. Median total commit time decreased by 11.95% and median maximum
commit latency decreased by 0.79%. Median recorded `sync_total_ns` increased by
0.99% and median maximum sync latency increased by 4.64%; the improvement comes
from avoiding unused in-memory index work rather than weakening durability.

The read guard passed. Across 25 measured 64-reader intervals per variant on
one byte-identical buffered dataset, median W3 throughput increased from
6,511,003.081 to 6,885,981.315 operations/s, a 5.76% improvement, and median
p99 decreased from 7,583 to 7,000 nanoseconds, a 7.69% improvement. Median
whole-process W3 RSS was neutral at 214,515,712 versus 214,482,944 bytes after
both variants had materialized the index. This remains above 1.25 times the
pinned RocksDB 64-reader reference. Buffered open-to-first-read median improved
slightly from 324,405,958 to 321,938,209 nanoseconds; lazy initialization moved
the same required build work into the measured first lookup without adding a
regression.

Every final W2 warm-up and measured run produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`, main-file
hash `f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`, and WAL
hash `112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68`.
Main-file and WAL lengths remained 65,486,848 and 57,963,128 bytes. Raw output
is retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-lazy-point-index-w2-w3-ab`.

A focused 16-thread race test initializes the index concurrently, commits an
update and insertion afterward, and verifies both through the initialized
index. The complete all-target, all-feature test suite, Clippy with warnings
denied, `cargo check`, format check, and diff check passed. Persistent bytes,
snapshot semantics, and the durability protocol are unchanged.
