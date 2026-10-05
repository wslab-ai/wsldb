# Concurrent RocksDB group-commit preregistration

Date: 2026-10-04. This routing contract is committed before implementing the
RocksDB adapter or collecting measurements.

## Purpose

WSLDB's accepted automatic group commit reached 1,973.986 durable
transactions/s with 16 callers and shared 4,096 transactions across 328 WAL
syncs. That establishes the local mechanism only. RocksDB also groups
overlapping writers, so a direct concurrent comparison is required before
claiming a scalability advantage.

The installed Homebrew RocksDB 10.10.1 library is used to avoid rebuilding a
large dependency on the laptop. This is a short routing diagnostic, not a
replacement for the pinned RocksDB 11.8.1 release matrix.

## Method

Add a dependency-free RocksDB adapter and a single-mode switch to the existing
WSLDB group benchmark. For each engine:

- create a fresh empty database before the measured interval;
- start 16 callers together and insert 4,096 unique 16-byte keys with
  deterministic 100-byte values;
- require synchronous WAL durability for every returned call;
- keep a 1 GiB write/memtable budget so foreground checkpoint or flush work is
  not moved into the active interval;
- wait for every caller before stopping the clock;
- verify every expected key and value after the interval;
- report active transactions/s, WAL synchronization count and histogram where
  exposed, WAL bytes, maximum RSS, and final logical storage;
- retain executable hashes and raw process output outside the repository.

Run one unreported warm-up per engine followed by five measured alternating
pairs. Pair order is WSLDB/RocksDB, RocksDB/WSLDB, WSLDB/RocksDB,
RocksDB/WSLDB, WSLDB/RocksDB.

## Interpretation

The result is accepted as a routing win only if:

- every run verifies all 4,096 byte-identical values;
- WSLDB wins at least four of five throughput pairs;
- WSLDB median throughput is at least 10% higher;
- WSLDB median WAL sync count is no more than 110% of RocksDB's;
- WSLDB maximum RSS is no more than RocksDB's;
- no engine has unmeasured foreground work after the timer.

Failure of any correctness check invalidates the experiment. Failure of a
performance gate records a negative result and leaves production code
unchanged. Passing this diagnostic would establish only the concurrent
strict-durability profile on this APFS host; it would not establish general
superiority or close the Linux and pinned-version release gates.

## Result

Negative under the complete frozen gate; production code is unchanged.

The adapter implementation commit was
`51966d75c72d54f45fc9ced0de29f78186365b57`. Executable SHA-256 values were:

- WSLDB group benchmark:
  `202d2fb5c21e3c19dd1f158c8983904fac284641103e3c4dfb2f6a09665d32b8`;
- RocksDB group adapter:
  `e7510aac2bd6557c2a84418b43861fe17d8a06efff8e4959abc2f1cd74f6dba4`.

Every WSLDB run ended at transaction 4,096 with logical SHA-256
`cbd8ba5aeeb849af1dc5ba4c266622880cf776788942a74e7044c16a73d4b65d`.
Every RocksDB run independently read and byte-compared all 4,096 expected keys
and values and observed exactly 4,096 entries.

| Median metric | WSLDB | RocksDB 10.10.1 | WSLDB relative result |
| :--- | ---: | ---: | ---: |
| Active throughput | 2,101.959 tx/s | 1,477.730 tx/s | 42.24% higher |
| WAL sync count | 322 | 495 | 34.95% fewer |
| WAL bytes | 802,816 | 493,364 | 62.72% more |
| Maximum RSS | 25,001,984 bytes | 10,780,672 bytes | 2.32 times larger |
| Final logical storage | 811,008 bytes | 556,572 bytes | 45.71% larger |

WSLDB won all five throughput pairs by 39.40%, 34.94%, 56.83%, 41.49%,
and 50.03%. Its median sync count was 65.05% of RocksDB's, so automatic
foreground grouping is effective and passed the throughput, pair, and sync
gates. No caller or mutation remained after either measured interval.

The complete candidate still fails because WSLDB maximum RSS exceeded
RocksDB's in every run. The cause is concrete: this diagnostic intentionally
configured a 1 GiB buffer limit, and `new_buffer_key_filter` eagerly sized a
10-bit-per-entry filter for the entire estimated limit. That reserves about
20 MiB even though the workload contains only 4,096 unique keys. RocksDB's
1 GiB memtable limit does not eagerly reserve its full capacity.

WSLDB also writes a larger WAL record for every independently checksummed
transaction. That byte overhead did not prevent the throughput win, but it
explains the storage result and remains a separate format-efficiency debt.

The next candidate may replace the eagerly maximum-sized write-buffer filter
with a bounded growing filter keyed by the actual unique-key count. It must
preserve the no-false-negative invariant and pass causal RSS, throughput,
persisted-byte, recovery, and corruption gates before acceptance. This result
must not be retroactively reclassified by lowering the configured buffer or
weakening the RSS gate.

Raw outputs and exact binaries are retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-concurrent-rocksdb-group`.
Disposable per-run database directories were moved to the system Trash after
their recorded sizes and verification results were retained.
