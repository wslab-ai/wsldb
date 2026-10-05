# Buffered checkpoint page writes

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

W2 sampling identifies positioned page writes as one of the dominant checkpoint
costs. The empty-tree bulk loader currently issues one positioned write for
every encoded 4 KiB page even though those pages are allocated and emitted in
strict physical order.

The candidate will buffer at most 256 KiB, or 64 pages, inside
`StreamingCommitContext` and emit each complete buffer with one positioned
write. The checkpoint must flush the final partial buffer before data sync.
Page IDs, encoded page bytes, metadata, WAL bytes, checksums, and logical order
remain unchanged. The direct copy-on-write writer is outside this experiment.

Crash-injection builds will flush before the existing per-page crash boundary
so the crash matrix retains its current observation granularity. Ordinary
production builds may group those writes.

## Frozen acceptance gate

The causal A/B uses one-million 16-byte keys, 100-byte values, 10,000 mutations
per synchronous commit, a 64 MiB durable write buffer, and 65,536 decoded cache
pages. It runs one warm-up and five measured fresh-file processes per variant,
alternating measured variants.

The candidate is accepted only if median active W2 throughput improves by at
least 3%, median maximum RSS grows by no more than 2%, every main-file, WAL, and
logical hash matches, the checkpoint crash/fault tests pass, and the complete
formatting, test, Clippy, check, and diff-check gate passes. Maximum commit and
sync latency are reported. Thresholds will not be changed after results are
observed.

## Causal A/B result

The final candidate passed every frozen threshold. One warm-up and five
measured fresh-file processes were run per variant, with measured variants
alternated.

| Metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Median active throughput | 398,404 ops/s | 431,802 ops/s | +8.38% |
| Median maximum commit latency | 539.3 ms | 341.3 ms | -36.71% |
| Median total sync time | 934.7 ms | 936.0 ms | +0.15% |
| Median maximum sync latency | 46.8 ms | 44.3 ms | -5.35% |
| Median maximum RSS | 177,012,736 B | 177,127,424 B | +0.06% |

All ten measured main files had SHA-256
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`.
All ten WAL files had SHA-256
`112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68`.
Every measured and warm-up logical hash was
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
Main and WAL lengths also matched at 65,486,848 and 57,963,128 bytes.

The final implementation defers branch-cache admission until the complete
positioned batch write succeeds. This prevents an I/O failure from leaving an
unwritten page ID resident in the cache. Crash-injection builds retain the
per-page write boundary described above.

Raw process output, resource reports, hashes, and sizes are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-buffered-checkpoint-pages-final-w2-ab`.

## Fresh RocksDB diagnostic

The same pinned official RocksDB 11.8.1 binary was run immediately afterward
with one warm-up and five fresh databases. RocksDB's median active
`filluniquerandom` throughput was 477,904 operations/s versus WSLDB's 431,802,
so RocksDB remains 1.11 times faster, or 10.68%. Median peak RSS was 114,835,456
bytes for RocksDB versus 177,127,424 bytes for WSLDB, a 1.54-times WSLDB gap.
RocksDB used 121,978,880 bytes after flush and compaction versus WSLDB's
123,449,976-byte main-plus-WAL state.

WSLDB's shorter 3.31-second median whole-process time is not treated as a win:
the RocksDB command explicitly flushed and waited for compaction after its
active interval, while WSLDB retained its durable WAL tail. RocksDB raw output
and sizes are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-buffered-pages-rocksdb-w2-1m`.

## Correctness verification

Default-feature checkpoint tests and the complete final-source all-features
gate passed:

- `cargo fmt --all -- --check`
- `cargo test --all-targets --all-features`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo check --all-targets --all-features`
- `git diff --check`
