# Borrowed cached-page inspection

Date: 2026-10-03. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.

## Change

Point reads previously cloned an `Arc<Node>` at every tree level. Shared-cache
hits also made another clone for the thread-local cache, so a successful lookup
performed atomic reference-count increments and decrements even though the
reader only needed the node for the duration of one comparison.

The page reader now supports closure-scoped inspection. The shared and local
caches keep the node alive while the closure runs, and the point-read path
copies only the requested value. Overflow traversal still occurs after the
cached node borrow ends. The file format, cache admission policy, durability
path, and public API are unchanged.

## Method

- Baseline: `0bd73fd` (`docs: refresh comparison after harness correction`).
- Candidate: `3fa289a` (`perf: inspect cached pages without arc churn`).
- Existing database: 1,000,000 entries, 16-byte keys, 100-byte values.
- Workload: 1,000,000 random point reads, 50% present and 50% absent.
- Readers: 16.
- Decoded-page cache capacity: 65,536 pages.
- Five independent processes per variant, alternated by process number.
- Each process performed one warm-up and five measured runs. The table reports
  the median measured run from each process.

Raw stdout, stderr, commands, host metadata, repository state, and database
checksum are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-borrowed-page-inspection`.

## Results

| Variant | Process medians, million ops/s | Median million ops/s | Median p99 | Median max RSS |
| --- | --- | ---: | ---: | ---: |
| Baseline | 6.086, 5.862, 5.751, 6.526, 5.718 | 5.862 | 4.500 us | 188,416,000 B |
| Candidate | 8.170, 8.714, 8.554, 10.678, 9.823 | 8.714 | 3.500 us | 188,678,144 B |

The candidate increased process-median throughput by 48.65% and reduced p99
by 22.22%. Median maximum RSS increased by 0.14%, which is treated as neutral.
Every measured candidate process was faster than every measured baseline
process.

## Verification

The change adds a focused cache test proving that a missing node is loaded once
and reused by the inspection path. `cargo fmt --all -- --check`,
`cargo test --all-targets`, `cargo clippy --all-targets -- -D warnings`,
`cargo check --all-targets --all-features`, and `git diff --check` passed.
