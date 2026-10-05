# Bounded latency percentile observability — 2026-10-04

## Subject

- Commit: `2bf4ef498cb71b07d6416b59927a519ec71522e8`.
- Host: Apple M1 Max, macOS 26.6.2, APFS.
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`.
- Public API contract SHA-256:
  `fc6f67b9b334a64e7b473a454c7cea79f3fa9cb62ff11be1d1b4cc4775c10992`.

## Contract

Every `LatencyStats` source now reports p50, p95, p99, and p99.9 in addition
to count, total, last, and maximum nanoseconds. The implementation uses a
fixed logarithmic histogram with 16 buckets per power of two and exact buckets
for zero through 15 nanoseconds. It reports each selected bucket's inclusive
upper bound, so a non-zero estimate is never below the recorded nearest-rank
value and is at most 6.25% above it.

Recording performs one additional relaxed atomic increment and no allocation.
All seven runtime histograms together are statically checked to occupy no more
than 64 KiB. Snapshot collection is bounded by 976 counters per histogram and
occurs only through `Database::operational_stats`, which already performs a
full uncached database verification.

The WSLDB benchmark report now emits `wal_sync_p50_ns`, `wal_sync_p95_ns`,
`wal_sync_p99_ns`, and `wal_sync_p999_ns`. This closes the earlier
instrumentation gap between end-to-end mutation tails and the physical WAL
synchronization tail. It does not change the database or WAL format, commit
ordering, synchronization primitive, or recovery behavior.

## Verification

The following checks passed with at most two Cargo build jobs:

- `cargo fmt --all -- --check`;
- `cargo check --all-targets --all-features`;
- `cargo clippy --all-targets --all-features -- -D warnings`;
- `cargo test --all-targets --all-features`: 205 tests, including crash,
  injected-I/O, corruption, multi-process, format-fixture, group-commit,
  public-API, and power-loss-probe tests;
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`;
- `scripts/release-audit.sh`;
- `git diff --check`.

Focused tests cover empty histograms, nearest-rank percentiles, the complete
`u64` duration domain including `u64::MAX`, the 6.25% error bound, the bounded
runtime footprint, a real direct commit, and a one-sync durable transaction
group. The benchmark target also compiled through the all-target check and
test suite.

## Interpretation

This is production observability and diagnostic capability, not evidence of a
throughput improvement. No sustained workload was rerun on the active laptop.
The next frozen W6 comparison can now determine directly whether WSLDB's
mutation p99 deficit is in stable-storage synchronization or in the surrounding
commit phases without inferring a distribution from only total and maximum
latency.
