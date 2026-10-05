# Unified buffered write path

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

Three byte-preserving experiments reduced independent portions of the durable
write path but missed their individual or partial-compound throughput gates. A
current-generation negative filter produced +3.29% W2, direct single-allocation
WAL encoding previously produced +1.43%, and a borrowed allocation-light bulk
loader reduced maximum checkpoint latency by 9.06%. The filter plus direct WAL
compound produced +3.08% and remained 1.10% below the separately pinned
RocksDB result. All candidate source was removed after each failed gate.

The unified candidate will apply all three mechanisms as one pipeline:

- definite-negative buffered-key probes skip the ordered version map;
- each WAL record is encoded directly into one exactly sized allocation;
- checkpoint bulk loading consumes borrowed keys and values, validates order
  against the retained leaf tail, and reuses leaf vector capacity.

The filter is updated under the authoritative state write lock after the WAL is
durable, rebuilt by recovery, and replaced with a preallocated empty filter at
checkpoint publication. Positive filter answers remain inconclusive. WAL
checksums, record ordering, page partitioning, sync order, transaction and
snapshot semantics, file bytes, and all persistent-format fields remain
unchanged. No work may be moved beyond the measured process lifetime.

## Frozen acceptance gate

The causal W2 A/B uses the one-million-key, 100-byte-value, 10,000-operation
commit, 64 MiB durable-buffer, 65,536-page-cache profile. Each variant gets one
unreported warm-up followed by five alternating fresh-file processes. Median
active throughput must improve by at least 4% and median maximum commit latency
must decrease by at least 5%. Median maximum RSS may regress by no more than 2%
and median maximum sync latency may regress by no more than 5%.

Every run must produce the same logical hash, main-file hash and length, and
WAL hash and length. Total commit and sync latency, write amplification, and
whole-process elapsed time are reported. Focused filter, recovery, checkpoint,
bulk ordering/ownership, WAL round-trip, fixture, truncation, and corruption
tests must pass. If the causal gate passes, the final binary receives a fresh
same-session paired RocksDB W2 comparison; matching or exceeding RocksDB is a
diagnostic outcome, not a substitute for the causal gate or a general
superiority claim. The complete formatting, all-target all-feature test,
Clippy, check, and diff-check gate must pass. Thresholds will not change after
candidate results are observed.

## Result

Accepted. Across five alternating measured processes, median active W2
throughput increased from 521,695.036 to 557,438.783 operations/s, a 6.85%
improvement. Median maximum commit latency decreased from 263,209,167 to
230,915,625 nanoseconds, a 12.27% improvement. Both primary gates passed.

Median total commit time decreased by 7.67%, median total sync time decreased
by 5.37%, and median whole-process elapsed time decreased from 2.80 to 2.69
seconds. Median maximum sync latency increased by 0.33%, within the 5%
guardrail. Median peak RSS decreased from 128,221,184 to 127,664,128 bytes, a
0.43% improvement. Write amplification remained 1.651.

All ten measured processes produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`, main-file
hash `f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`, and WAL
hash `112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68`.
Main-file and WAL lengths remained 65,486,848 and 57,963,128 bytes. Raw causal
output and both binaries are retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-unified-buffered-write-path-w2-ab`.

The complete all-target all-feature test suite passed, including crash,
partial-I/O, recovery, multiprocess, compaction, backup, format-fixture, and
model-based checks. Clippy with warnings denied, `cargo check`, format check,
and diff check passed. The accepted implementation is commit `55153a6`.

The required fresh paired RocksDB diagnostic also completed. In that session,
WSLDB reached 524,188.733 operations/s versus RocksDB 11.8.1 at 491,990
operations/s, a 6.55% WSLDB active-throughput lead. This is a narrow W2 result,
not a general superiority claim: WSLDB's median maximum durable-batch latency
remained 6.42 times higher, peak RSS 7.29% higher, and logical storage 1.23%
higher. Separate evidence records the exact build and postconditions.
