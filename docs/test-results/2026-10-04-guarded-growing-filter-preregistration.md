# Guarded growing write-buffer filter preregistration

Date: 2026-10-04. This is a new contract committed after the first growing
filter was rejected and removed, but before this candidate is implemented.

## Observation

The rejected geometric filter reduced concurrent-group median RSS by 83.89%
and preserved exact persisted output, proving that eager maximum sizing causes
the measured memory debt. It also added unique-key accounting and capacity
preparation to every commit. Median throughput fell 5.17%, outside its 3%
allowance.

In the measured workload only 4,096 keys enter a filter initially capable of
holding 65,536. No growth work is necessary on that path.

## Candidate

Retain geometric rebuilds and their pre-sync allocation rule, but guard them
with a cheap capacity upper bound.

- Initially reserve for at most 65,536 unique keys or the smaller estimated
  generation maximum.
- Compare current exact-map length plus the batch mutation count with filter
  capacity. When it fits, perform no unique-key accounting and no filter
  preparation beyond the state write guard already required before WAL append.
- Only when the conservative upper bound crosses capacity, count exact new
  keys, geometrically rebuild before WAL sync, and then publish filter bits
  without allocation after sync.
- Repeated hot-key updates must not grow the filter. Replay and snapshot paths
  may rebuild because they are already fallible pre-publication operations.
- Persisted bytes, API, transaction ordering, and checkpoint behavior remain
  unchanged.

## Frozen acceptance gate

The same focused no-false-negative, growth, duplicate-key, recovery, snapshot,
group, and failure tests are required, followed by the complete all-feature
test suite, all-target check, Clippy with warnings denied, formatting, and diff
check.

Use the exact baseline binary and the same 4,096-transaction, 16-caller causal
method as the rejected candidate: one warm-up and five alternating measured
pairs. The candidate must retain the exact logical SHA-256 and 802,816 WAL
bytes, reduce median maximum RSS by at least 40%, remain below RocksDB's
10,780,672-byte prior median RSS, regress median throughput by no more than 3%,
and use no more than 110% of baseline median WAL syncs.

If it passes, run five fresh alternating candidate/RocksDB pairs. Closing the
earlier concurrent comparison additionally requires at least four candidate
pair wins, at least 10% median throughput lead, no more than 110% of RocksDB's
median sync count, and no more median maximum RSS.

Failure removes the candidate. Passing remains a narrow APFS result and cannot
replace pinned RocksDB 11.8.1 or Linux evidence.

## Result

Accepted. The implementation commit was
`485ecb4889cfa9699695cdb31d5023625fb74b6b`. The exact executable SHA-256
values were:

- baseline WSLDB:
  `202d2fb5c21e3c19dd1f158c8983904fac284641103e3c4dfb2f6a09665d32b8`;
- candidate WSLDB:
  `b0f2b76a9c3debb5f18a959643cd04f729e8edc95db5af009a2e21207c81e175`;
- RocksDB adapter:
  `e7510aac2bd6557c2a84418b43861fe17d8a06efff8e4959abc2f1cd74f6dba4`.

Every WSLDB process ended at transaction 4,096 with logical SHA-256
`cbd8ba5aeeb849af1dc5ba4c266622880cf776788942a74e7044c16a73d4b65d`,
802,816 WAL bytes, and 811,008 total bytes. Every RocksDB process independently
verified all 4,096 expected values and exactly 4,096 entries.

### Causal result

| Median metric | Baseline | Candidate | Candidate result |
| :--- | ---: | ---: | ---: |
| Active throughput | 1,954.475 tx/s | 1,925.585 tx/s | 1.48% lower |
| WAL sync count | 342 | 351 | 2.63% higher |
| Maximum RSS | 24,936,448 bytes | 4,128,768 bytes | 83.44% lower |

The throughput change remained within the frozen 3% guardrail, sync count was
102.63% of baseline within the 110% cap, and candidate RSS was 61.63% below
the prior RocksDB median. Exact logical and persisted output was unchanged, so
the causal gate passed.

### Fresh RocksDB result

| Median metric | WSLDB | RocksDB 10.10.1 | WSLDB relative result |
| :--- | ---: | ---: | ---: |
| Active throughput | 1,620.256 tx/s | 1,287.200 tx/s | 25.87% higher |
| WAL sync count | 351 | 496 | 29.23% fewer |
| Maximum RSS | 4,014,080 bytes | 10,764,288 bytes | 62.71% lower |
| WAL bytes | 802,816 | 493,376 | 62.72% more |
| Final logical storage | 811,008 bytes | 556,587 bytes | 45.71% larger |

WSLDB won all five fresh throughput pairs by 45.03%, 37.17%, 24.50%, 39.29%,
and 25.02%. It passed the pair, median-throughput, sync-count, and RSS gates.
No caller or mutation remained after a measured interval. The candidate is
therefore accepted.

Before measurement, 63 unit tests and every integration, crash, I/O-failure,
maintenance, and multi-process test passed, followed by three documentation
tests, all-target all-feature check, Clippy with warnings denied, formatting,
and diff check. Growth allocation remains before WAL synchronization;
post-sync filter insertion remains allocation-free.

This closes only the Homebrew RocksDB concurrent strict-durability diagnostic.
The 62.72% WAL-byte and 45.71% final-storage deficits remain explicit debts,
and the result does not replace the pinned RocksDB 11.8.1 or Linux matrix.

Raw causal output is retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-guarded-growing-filter`;
fresh comparison output is retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-guarded-growing-filter-rocksdb`.
Disposable per-run databases were moved to the system Trash after their sizes
and verification results were recorded.
