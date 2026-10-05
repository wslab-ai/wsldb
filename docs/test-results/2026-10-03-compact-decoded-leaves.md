# Compact decoded leaf storage

Date: 2026-10-03. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is a local optimization A/B, not a cross-platform release claim.

- Baseline: `7684e41da033b181354ae61156c8bce3dfbcfe92`
- Candidate: `d3656633b86df74201fa2172f9e026284d4d7b0f`
- Dataset: 1,000,000 entries, 16-byte keys, 100-byte values
- Load batch: 10,000 operations
- Cache capacity: 65,536 decoded pages
- Build: release benchmark profile
- Raw evidence:
  `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-compact-decoded-leaves`

The candidate stores immutable decoded keys and inline values in exact-size
boxed slices instead of capacity-bearing vectors. This does not change the
file encoding or public API.

## Why cache capacity alone was not reduced

A baseline cache sweep used the same database and 16-reader W3 workload. Each
row is one process with one warm-up and five measured runs.

| Cache pages | Resident pages | Median ops/s | Median p99 | Max RSS |
| ---: | ---: | ---: | ---: | ---: |
| 4,096 | 4,096 | 440,173 | 353.458 us | 84,918,272 B |
| 8,192 | 8,192 | 467,726 | 358.541 us | 115,015,680 B |
| 16,384 | 16,384 | 618,511 | 293.958 us | 176,193,536 B |
| 32,768 | 27,205 | 5,480,384 | 4.500 us | 258,228,224 B |
| 65,536 | 27,205 | 5,918,618 | 4.500 us | 258,162,688 B |

Reducing residency to approach RocksDB's measured RSS would therefore discard
the working set and lose most W3 throughput. The accepted change instead
reduces bytes per decoded entry while retaining the full working set.

## A/B results

| Workload | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| W1 sequential load | 373,192 ops/s | 373,052 ops/s | -0.04% |
| W1 maximum RSS | 218,398,720 B | 188,514,304 B | -13.68% |
| W3 point reads, 16 readers | 5,649,029 ops/s | 5,838,294 ops/s | +3.35% |
| W3 median-process p99 | 4.667 us | 4.646 us | -0.44% |
| W3 maximum RSS | 258,138,112 B | 228,573,184 B | -11.45% |
| W4 ranges of 100 | 154,240 ranges/s | 171,638 ranges/s | +11.28% |
| W4 median-process p99 | 7.791 us | 7.000 us | -10.15% |
| W4 maximum RSS | 209,551,360 B | 179,961,856 B | -14.12% |

W1 uses the median of five fresh-file processes. W3 first takes the median of
five measured runs within each process, then the median of ten process
medians. W4 applies the same aggregation across five processes. Engine order
was alternated where applicable. The W1 throughput difference is treated as
neutral; the memory reductions and W3/W4 improvements are retained.

## Verification

- `cargo fmt --all -- --check`
- `cargo test --all-targets`
- `cargo clippy --all-targets -- -D warnings`
- `cargo check --all-targets --all-features`
- `git diff --check`
