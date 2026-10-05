# Sharded point-read latency samples

Date: 2026-10-03. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is a benchmark-harness correction, not an engine-speed claim.

- Baseline: `1b5271800f01f5bcd43f8d0b0c0e1f35169ca27a`
- Candidate: `52873658a777d70ce7193f9b86f5e4a7521d0fcb`
- Database: 1,000,000 entries, 16-byte keys, 100-byte values
- Workload: 1,000,000 random reads, 16 readers, five measured intervals
- Cache capacity: 65,536 decoded pages
- Raw evidence:
  `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-sharded-latency-samples`

The old W3 harness retained one million `Duration` values, copied them into a
combined vector before stopping the reported interval, and then allocated a
second `u128` vector to sort. The corrected harness stores nanoseconds as
per-worker `u64` shards, stops timing when the reader workers finish, sorts
outside that interval, and finds exact global percentile ranks by binary
searching the sorted shards. It neither samples nor approximates latencies.

| Metric | Baseline median | Candidate median | Change |
| --- | ---: | ---: | ---: |
| Reported interval throughput | 5,556,030 ops/s | 5,956,411 ops/s | +7.21% |
| p50 | 1.459 us | 1.459 us | 0.00% |
| p95 | 2.542 us | 2.500 us | -1.65% |
| p99 | 4.417 us | 4.125 us | -6.61% |
| p99.9 | 5.833 us | 5.292 us | -9.28% |
| Process maximum RSS | 228,622,336 B | 188,465,152 B | -17.57% |

Each result is the median of five process medians; each process contains five
measured runs after a warm-up. The apparent throughput gain removes harness
aggregation from the operation interval rather than changing the database.
The latency-tail differences are treated as scheduler noise; the unchanged p50
and exact-rank algorithm confirm compatible measurement semantics.

## Verification

- `cargo fmt --all -- --check`
- `cargo test --all-targets`
- `cargo clippy --all-targets -- -D warnings`
- `cargo check --all-targets --all-features`
- `git diff --check`
