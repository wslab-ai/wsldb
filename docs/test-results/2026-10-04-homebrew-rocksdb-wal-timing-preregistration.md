# Homebrew RocksDB WAL timing preregistration

Date: 2026-10-04. This routing contract is committed before building the
adapter or measuring either engine.

## Purpose

The accepted compound buffered-preflight candidate reduced short W6 preflight
by 69.33% and throughput improved by 6.52%. Stable-storage synchronization now
dominates successful WSLDB commit time. The pinned RocksDB 11.8.1 static build
is no longer present locally, while Homebrew RocksDB 10.10.1 is already
installed with headers and libraries.

This diagnostic compares the synchronization mechanisms without rebuilding
RocksDB. Because the RocksDB version is not the frozen release-gate version,
the result may only route engineering work. It cannot establish parity or
replace any production-readiness comparison.

## Method

Build the existing dependency-free sustained adapter against the installed
Homebrew RocksDB 10.10.1 library. Load one million byte-identical 16-byte keys
and deterministic incompressible 100-byte values into each engine, verify the
same logical SHA-256, and retain these load images as clone sources.

Run one unreported warm-up and three measured alternating W6 pairs. Each
process uses:

- a two-second active interval;
- Zipf exponent 1.2 and the frozen 80/15/5 operation mix;
- a 256 MiB cache budget;
- a 64 MiB WSLDB durable buffer;
- synchronous WAL durability for every mutation;
- RocksDB's opt-in `WAL_FILE_SYNC_MICROS` statistics and WSLDB's exact WAL
  phase counters;
- full final checksum/logical verification and exactly one million entries.

Record executable hashes, per-run active throughput, mutation p99, WAL sync
count, mean and p99 synchronization time, maximum RSS, and final logical
storage. Raw files remain outside the repository under the benchmark-evidence
directory.

## Interpretation

- If RocksDB's median mean WAL synchronization is at least 8% lower, inspect
  its concrete WAL write and synchronization path before another WSLDB CPU
  optimization.
- If median means are within 8%, remaining short throughput differences are
  not attributed to a different durability primitive; route work to measured
  encoding, write, publication, and concurrency costs.
- If the result changes sign with order or any logical verification differs,
  classify it as inconclusive and make no production change.

No production code or public API is changed by this diagnostic.

## Result

Inconclusive by the frozen sign-stability rule; no production code changed.

The diagnostic ran on macOS 26.6.2 / Darwin 25.6.0 arm64. The source commit
was `2130b84183ea6c765a05e3fe6e14d1442bd34c47`. Executable SHA-256 values were:

- WSLDB: `1027dceed2ded19c9df2d0d6473ac83e494289d7f745f69cfb9ce54f60aeeefd`;
- RocksDB adapter: `adbaa0eff2687017f3ab596418f030185b435e70c6132f018225be4cf5b05dc6`;
- Homebrew RocksDB 10.10.1 dynamic library:
  `613aa4dcaf331586beaddbd2d31d918e19141eeeed319dfb18a71b67560cd4c2`.

Both load images contained exactly 1,000,000 entries and produced the same
logical SHA-256
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
Every measured process retained exactly 1,000,000 entries and completed its
independent logical verification.

| Median metric | WSLDB | RocksDB 10.10.1 | WSLDB relative result |
| :--- | ---: | ---: | ---: |
| Active throughput | 804.515 ops/s | 770.055 ops/s | 4.48% higher |
| Overwrite p99 | 12.167 ms | 10.577 ms | 15.04% worse |
| Delete/recreate p99 | 19.686 ms | 18.620 ms | 5.72% worse |
| Mean WAL sync | 4.716 ms | 4.907 ms | 3.89% lower |
| WAL sync count | 391 | 372 | workload-rate dependent |
| Maximum WAL sync | 17.219 ms | 16.702 ms | 3.10% worse |
| Maximum RSS | 142,409,728 bytes | 273,170,432 bytes | 47.87% lower |
| Final logical storage | 122,571,564 bytes | 122,032,440 bytes | 0.44% larger |

WSLDB throughput was 19.32% lower in pair 1, 4.48% higher in pair 2, and
8.53% higher in pair 3. Mean WAL synchronization changed sign in the same
session: WSLDB was 15.11% slower, 4.93% faster, and 5.73% faster respectively.
The median synchronization means are within the preregistered 8% band, but the
pair signs are not stable. The result therefore cannot establish a throughput
win or a stable-storage advantage for either engine.

The current WSLDB counters expose exact sync count, total, and maximum, but not
a synchronization histogram, so the preregistered WSLDB sync p99 could not be
reported. Mutation p99 remains available end to end. This deviation further
limits the diagnostic and does not justify adding production instrumentation
solely to rescue the experiment.

The routing conclusion is still useful: RocksDB did not show a repeatable
lower mean durable-sync cost. A different tree or compaction algorithm cannot
remove the one physical barrier from an isolated strict-durability commit.
Further work should measure concurrent group commit against RocksDB and use an
adaptive buffered checkpoint/compaction policy to address write and space
amplification. The STB-epsilon tree in
[SplinterDB](https://www.usenix.org/conference/atc20/presentation/conway) and
the log/storage coordination in
[TRIAD](https://www.usenix.org/conference/atc17/technical-sessions/presentation/balmau)
are relevant design references for that merge path; neither result is treated
as evidence for WSLDB until implemented and measured under its own frozen
contract.

Raw stdout, stderr, size records, binaries, and initial images are retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-homebrew-rocksdb-wal-timing`.
Per-run database clones were disposable measurement fixtures, not evidence.
