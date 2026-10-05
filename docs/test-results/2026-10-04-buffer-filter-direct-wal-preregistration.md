# Compound buffered filter and direct WAL encoding

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or compound
measurement.

## Observation and candidate

Two isolated candidates produced positive but individually insufficient W2
results. A current-buffer negative filter improved throughput by 3.29%, while
an earlier single-allocation direct WAL encoder improved it by 1.43%. They act
on disjoint portions of every commit: ordered-map negative lookup and WAL
payload construction. Both were completely removed after missing their own
frozen gates.

The compound candidate will restore the generation-scoped probabilistic filter
with the same fail-safe semantics and encode each WAL record directly into one
exactly sized output allocation. The encoder will reserve the complete record,
leave space for the header, write the canonical payload once, fill its header
checksums, and append the existing footer. It must produce the exact existing
bytes. The filter's definite negatives skip only the ordered-map lookup;
positives remain inconclusive and use the authoritative map.

Durable publication, recovery, group commit, checkpoint reset, transaction
ordering, checksums, sync order, entry counts, point reads, page bytes, WAL
bytes, and every persistent-format field remain unchanged. No work may be
deferred beyond the measured commit or process lifetime.

## Frozen acceptance gate

The causal W2 A/B uses the one-million-key, 100-byte-value, 10,000-operation
commit, 64 MiB durable-buffer, 65,536-page-cache profile. Each variant gets one
unreported warm-up followed by five alternating fresh-file processes. Median
active throughput must improve by at least 4%. Median maximum RSS may regress
by no more than 2%, and median maximum commit and sync latency may regress by no
more than 5%.

Every run must produce the same logical hash, main-file hash and length, and
WAL hash and length. Total commit and sync latency, write amplification, and
whole-process elapsed time are reported. Focused coverage must include filter
negative/insertion/replay/reset behavior and exact WAL round trips, fixtures,
truncation, and corruption detection. The complete formatting, all-target
all-feature test, Clippy, check, and diff-check gate must pass. Thresholds will
not be changed after candidate results are observed.

## Result

Rejected and removed. Median active W2 throughput increased from 535,600.973
to 552,103.960 operations/s, a 3.08% improvement that does not meet the frozen
4% threshold. Median total commit time decreased by 3.16%, median total sync
time decreased by 0.70%, and median whole-process elapsed time decreased from
2.74 to 2.69 seconds.

Median maximum commit latency improved slightly from 265,008,584 to
264,335,958 nanoseconds. Median maximum sync latency increased by 1.64%, within
the 5% guardrail. Median peak RSS decreased from 128,204,800 to 126,943,232
bytes, a 0.98% improvement. These secondary passes do not override the primary
throughput gate. For context only, the candidate median remained 1.10% below
the separately pinned 558,157-operations/s RocksDB W2 reference; this was not a
fresh paired RocksDB run.

All ten measured processes produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`, main-file
hash `f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`, and WAL
hash `112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68`.
Main-file and WAL lengths remained 65,486,848 and 57,963,128 bytes; write
amplification remained 1.651. Raw output and both binaries are retained outside
the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-buffer-filter-direct-wal-w2-ab`.

WAL round-trip, corruption, truncation, snapshot replacement, format-fixture,
filter replay/reset, and automatic-checkpoint tests passed, as did
`cargo check --all-targets`. Both candidate mechanisms and their candidate-only
test were then fully removed. Their non-additive result indicates that the
remaining W2 gap is dominated by commit I/O and synchronization rather than
these CPU allocation paths.
