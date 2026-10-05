# Packed decoded leaf cache

Date: 2026-10-03. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is a local optimization A/B, not a cross-platform release claim.

## Change

Immutable leaf pages previously decoded every key and inline value into a
separate heap allocation. A million-entry cache therefore retained roughly two
million small allocations in addition to the page structures.

Decoded leaves now keep their common key prefix, key suffixes, and inline
values in one byte arena with exact eight-byte slots. Point lookup compares the
common prefix once per candidate and then compares the suffix directly.
Mutable copy-on-write pages keep the owned representation and materialize a
packed leaf only when it is changed. The persisted encoding and public API are
unchanged.

## Method

- Baseline: `c0b1f09` (borrowed cached-page inspection, before packed leaves).
- Candidate: `8a5fd58` (packed leaves with direct prefix/suffix comparison).
- Existing database: 1,000,000 entries, 16-byte keys, 100-byte values.
- W3: 1,000,000 random point reads, 50% present, 16 reader threads.
- W4: 10,000 ordered ranges of 100 entries, one reader thread.
- Decoded-page cache capacity: 65,536 pages.
- Five independent processes per variant and workload, alternated by process.
- Each process performed one warm-up and five measured runs. Each table cell
  uses the median measured run from a process, then the median process.

Raw stdout, stderr, binary and database checksums are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-packed-leaf-prefix-compare`.

## Results

| Workload and metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| W3 throughput | 7.383 M ops/s | 8.222 M ops/s | +11.36% |
| W3 p99 | 4.125 us | 3.750 us | -9.09% |
| W3 maximum RSS | 188,661,760 B | 136,626,176 B | -27.58% |
| W4 throughput | 140,137 ranges/s | 141,718 ranges/s | +1.13% |
| W4 p99 | 11.666 us | 11.417 us | -2.13% |
| W4 maximum RSS | 179,961,856 B | 127,221,760 B | -29.31% |

The candidate improved throughput, p99 latency, and memory use in both
preregistered read workloads. The large RSS reduction agrees with the heap
profile that identified per-entry key and value allocations as the dominant
decoded-cache cost.

## Verification

The packed representation has focused tests for its exact slot size, inline
and overflow values, short lookup keys, keys above the leaf maximum, and
byte-identical persisted round trips. The following commands passed:

- `cargo fmt --all -- --check`
- `cargo test --all-targets`
- `cargo clippy --all-targets -- -D warnings`
- `cargo check --all-targets --all-features`
- `cargo test --all-targets --all-features`
- `git diff --check`
