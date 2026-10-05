# Buffered-key negative filter

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or candidate
measurement.

## Observation and candidate

The current W2 profile attributes 138 of 1,496 main-thread samples to buffered
presence checks. Before each durable append, every mutation searches the
growing ordered version map even when the key has never appeared in the current
WAL generation. The random unique-key workload makes almost all of those
searches negative.

The candidate will maintain a compact probabilistic negative filter for keys
that have appeared in the current write buffer. A definite negative skips the
ordered-map search. A positive remains inconclusive and follows the existing
version lookup, so false positives affect only speed and false negatives are
forbidden. Durable publication inserts keys into the filter under the same
state write lock as the version map. Recovery and WAL snapshot replay rebuild
it from durable records; a successful checkpoint installs a preallocated empty
filter for the next generation.

The filter is sized from the configured buffer budget at one expected entry per
64 encoded bytes. Overfilling can only increase false positives. WAL ordering,
entry-count calculation, snapshots, point-index behavior, sync order, page
bytes, WAL bytes, and every persistent-format field remain unchanged. No work
is moved beyond the measured commit or process lifetime.

## Frozen acceptance gate

The causal W2 A/B uses the one-million-key, 100-byte-value, 10,000-operation
commit, 64 MiB durable-buffer, 65,536-page-cache profile. Each variant gets one
unreported warm-up followed by five alternating fresh-file processes. Median
active throughput must improve by at least 4%. Median maximum RSS may regress
by no more than 2%, and median maximum commit and sync latency may regress by no
more than 5%.

Every run must produce the same logical hash, main-file hash and length, and
WAL hash and length. Total commit and sync latency, write amplification, and
whole-process elapsed time are reported. Focused tests must cover definite
negatives, inserted keys, replay, and a successful checkpoint generation
reset. The complete formatting, all-target all-feature test, Clippy, check,
and diff-check gate must pass. These thresholds will not be changed after
candidate results are observed.

## Result

Rejected and removed. Median active W2 throughput increased from 526,848.958
to 544,157.163 operations/s, a 3.29% improvement that does not meet the frozen
4% threshold. Median total commit time decreased by 3.57%, median total sync
time decreased by 0.41%, and median whole-process elapsed time decreased from
2.78 to 2.73 seconds.

Median maximum commit latency increased from 254,775,958 to 262,820,708
nanoseconds, a 3.16% regression, and median maximum sync latency increased by
4.58%; both remained inside their 5% guardrails. Median peak RSS increased from
128,352,256 to 129,728,512 bytes, a 1.07% increase within the 2% resource
guardrail. The throughput gate nevertheless remains authoritative.

All ten measured processes produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`, main-file
hash `f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`, and WAL
hash `112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68`.
Main-file and WAL lengths remained 65,486,848 and 57,963,128 bytes; write
amplification remained 1.651. Raw output and both binaries are retained outside
the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-buffer-key-filter-w2-ab`.

Focused filter replay/reset and successful automatic-checkpoint tests passed,
as did `cargo check --all-targets`. The implementation and candidate-only test
were then fully removed. The measured positive effect is suitable evidence for
a future compound hot-path candidate, but is not counted as an accepted gain.
