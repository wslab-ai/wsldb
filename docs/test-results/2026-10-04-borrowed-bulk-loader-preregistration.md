# Borrowed allocation-light bulk loading

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or candidate
measurement.

## Observation and candidate

The current one-million-key W2 profile spends visible checkpoint time in
`BulkLoader::flush_leaf`, page encoding, memory copying, and allocation. The
streaming checkpoint copies every key and value into temporary `Vec`s before
the bulk loader copies them again into the durable leaf representation. The
loader also keeps a second owned copy of the latest key solely to validate
ordering and discards the leaf vector allocation after every page.

The candidate will make `BulkLoader::push` accept borrowed key and value
slices, validate ordering against the last entry that the loader necessarily
retains, and reuse the leaf vector allocation after a page is encoded. Each
key and inline value will therefore be allocated and copied only for its final
leaf entry. The generic initial-load and buffered-checkpoint paths will use the
same interface.

Page partitioning, page encoding, checksums, transaction metadata, WAL bytes,
sync order, snapshot semantics, and the on-disk format remain unchanged. No
work may be deferred beyond the measured commit or process lifetime.

## Frozen acceptance gate

The causal W2 A/B uses the one-million-key, 100-byte-value, 10,000-operation
commit, 64 MiB durable-buffer, 65,536-page-cache profile. Each variant gets one
unreported warm-up followed by five alternating fresh-file processes. The
candidate must improve median active throughput by at least 2% and reduce
median maximum commit latency by at least 5%. Median maximum RSS may regress by
no more than 2%.

Every run must produce the same logical hash, main-file hash and length, and
WAL hash and length. Total commit and sync latency, maximum sync latency,
write amplification, and whole-process elapsed time are reported. Focused bulk
ordering and file-equivalence tests plus the complete formatting, all-target
all-feature test, Clippy, check, and diff-check gate must pass. These thresholds
will not be changed after candidate results are observed.

## Result

Rejected and removed. Across the five alternating measured processes, median
active W2 throughput changed from 537,647.464 to 536,703.725 operations/s, a
0.18% regression rather than the required 2% improvement. Median total commit
time increased by 0.26%, median total sync time increased by 0.88%, and median
whole-process elapsed time changed from 2.75 to 2.76 seconds.

The local mechanism did reduce median maximum commit latency from 257,319,791
to 234,011,000 nanoseconds, a 9.06% improvement, and median maximum sync
latency decreased by 3.47%. Median peak RSS was effectively neutral at
128,450,560 versus 128,417,792 bytes. Because the frozen gate required both
tail and throughput improvements, the candidate does not qualify.

All ten measured processes produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`, main-file
hash `f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`, and WAL
hash `112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68`.
Main-file and WAL lengths remained 65,486,848 and 57,963,128 bytes. Write
amplification remained 1.651. Raw output and the pre-candidate and candidate
binaries are retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-borrowed-bulk-loader-ab`.

The candidate's focused ordering/ownership test and `cargo check --all-targets`
passed before measurement. The source and candidate-only test were then fully
removed, leaving only this negative result. The result narrows the next work:
temporary input copies affect the individual checkpoint tail, but eliminating
them alone does not improve end-to-end W2 throughput.
