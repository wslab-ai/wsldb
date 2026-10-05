# Selective decoded-page write allocation

Date: 2026-10-03. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This is a local optimization A/B, not a cross-platform release claim.

- Baseline: `00459f4bafa848320ee75837940014c23b62fd19`
- Candidate: `24a15de65a012bdbaddbbdd5310ccb66691ce176`
- Values: 100 bytes; keys: 16 bytes
- Cache capacity: 65,536 decoded pages
- Build: release benchmark profile
- Raw evidence:
  `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-selective-write-cache`

The baseline admitted every newly written page into the decoded cache. The
candidate keeps branch pages, replacements, and ordinary split siblings hot,
but defers admission of new bulk/append leaf and overflow pages until their
first actual read. The file encoding, durability path, and public data API are
unchanged.

## Results

| Workload | Metric | Baseline median | Candidate median | Change |
| --- | --- | ---: | ---: | ---: |
| W1 sequential load, 1M, batch 10k | throughput | 360,752 ops/s | 363,422 ops/s | +0.74% |
| W1 sequential load, 1M, batch 10k | maximum RSS | 188,547,072 B | 13,107,200 B | -93.05% |
| W1 sequential load, 1M, batch 10k | resident pages | 27,511 | 583 | -97.88% |
| W2 random load, 100k, batch 10k | throughput | 65,094 ops/s | 65,008 ops/s | -0.13% |
| W2 random load, 100k, batch 10k | maximum RSS | 123,469,824 B | 123,305,984 B | -0.13% |
| W2 random load, 100k, batch 10k | cache misses | 0 | 271 | +271 |

Each result is the median of five fresh-file processes. The W1 and W2
throughput differences are treated as neutral. W1 retains only the small hot
branch set and one replacement leaf per append batch instead of filling the
cache with the whole write-only dataset. W2 retains its working set because
replacement leaves and both sides of ordinary leaf splits are marked hot; its
271 remaining misses did not produce a measurable median regression.

## Verification

- Focused cache-policy test proves bulk leaves start cold and replacements are
  readable from the shared cache without a miss.
- `cargo fmt --all -- --check`
- `cargo test --all-targets`
- `cargo clippy --all-targets -- -D warnings`
- `cargo check --all-targets --all-features`
- `git diff --check`
