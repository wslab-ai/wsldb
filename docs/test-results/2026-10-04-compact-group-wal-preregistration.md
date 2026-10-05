# Compact group WAL preregistration

Date: 2026-10-04. This contract is committed before the WAL codec or its tests
are changed.

## Observation

The accepted guarded write-buffer filter leaves the short concurrent durable
profile with a specific storage debt. WSLDB writes 802,816 WAL bytes for 4,096
single-mutation transactions, while Homebrew RocksDB 10.10.1 writes a median
493,376 bytes. Final storage is 811,008 versus 556,587 bytes. WSLDB nevertheless
wins all five throughput pairs and uses fewer syncs and less memory.

The difference is structural. `append_group` currently concatenates complete
independently framed records. Every logical transaction therefore repeats a
52-byte header and 16-byte footer even though the group already shares one
positioned write and one synchronization. LevelDB's documented log format
shares small physical-record framing, and RocksDB serializes write batches
inside a writer group. Those designs motivate the framing boundary only; this
candidate is an independent WSLDB codec and copies no external source code.

Primary references:

- LevelDB log format: <https://github.com/google/leveldb/blob/main/doc/log_format.md>
- RocksDB write-ahead log: <https://github.com/facebook/rocksdb/wiki/Write-Ahead-Log-%28WAL%29>
- RocksDB two-phase commit implementation and write grouping:
  <https://github.com/facebook/rocksdb/wiki/Two-Phase-Commit-Implementation>

## Candidate

Add a distinct compact physical group record while retaining the existing
transaction and snapshot records for backward readability and ordinary
single-transaction commits.

- One group header stores the first base transaction ID, final transaction ID,
  final entry count, logical transaction count, payload checksum, and header
  checksum. One footer stores the complete-record checksum and repeated length.
- Logical transaction IDs are implicit consecutive successors. Each logical
  transaction stores its resulting entry count and mutation count as canonical
  unsigned varints.
- Mutation prefix length, suffix length, and a tagged value length use canonical
  varints. Tag zero means tombstone; a present value is encoded as length plus
  one, preserving empty values. Key-prefix compression may continue across
  logical transaction boundaries inside the checksummed group.
- The decoder validates the complete group, every count and length, the final
  transaction ID, final entry count, key ordering within each transaction, and
  all checksums before yielding any logical record.
- A truncated final group is an incomplete tail and yields no transaction from
  that group. A complete corrupt group fails closed. A successful shared sync
  still makes every logical transaction durable before callers return.
- Empty, snapshot-containing, discontinuous, oversized, or internally
  inconsistent groups fail before any write. The one-GiB physical-record bound
  remains.

This is a pre-release compatibility change to the explicitly unversioned
development format. It does not introduce a numbered format. Previously
written individual and concatenated-group records remain readable. The
committed fixture is changed only if its encoded bytes actually exercise the
new group record.

## Frozen correctness gate

Focused tests must cover deterministic round trip, empty logical batches,
cross-transaction prefix reconstruction, every possible truncation, payload
and complete-record corruption, non-canonical and overflowing varints,
transaction-count/final-ID/final-entry mismatches, discontinuous input, tail
replay, reopen, checkpoint, partial write rollback, sync ambiguity, and process
crashes before and after the shared sync.

Then run the complete all-feature test suite, all-target all-feature check,
Clippy with warnings denied, formatting, documentation tests, and diff check.
No benchmark result can compensate for a correctness failure.

## Frozen performance gate

The exact accepted baseline executable is retained outside the repository with
SHA-256
`b0f2b76a9c3debb5f18a959643cd04f729e8edc95db5af009a2e21207c81e175`.
Use fresh databases and the same 4,096-transaction, 16-caller automatic-group
workload. Run one unreported warm-up per executable followed by five alternating
baseline/candidate pairs. Every run must finish at transaction 4,096 with
logical SHA-256
`cbd8ba5aeeb849af1dc5ba4c266622880cf776788942a74e7044c16a73d4b65d`.

The candidate is accepted only if:

- median WAL bytes fall by at least 25%;
- median throughput regresses by no more than 3%;
- median WAL sync count is no more than 110% of baseline;
- median maximum RSS is no more than 110% of baseline;
- all correctness and durability gates pass.

If accepted causally, run five fresh alternating candidate/RocksDB pairs using
the already committed adapter and method. Closing the measured storage debt
additionally requires WSLDB median final storage no larger than RocksDB's,
at least four throughput pair wins, at least a 10% median throughput lead, no
more than 110% of RocksDB's median sync count, and no more median maximum RSS.

Passing remains a narrow APFS/Homebrew RocksDB 10.10.1 result. It does not
replace the pinned RocksDB 11.8.1 or Linux release matrix and does not establish
general superiority.

## Result

Accepted. The implementation commit was
`76bcabe7dbf9b44f1ef1dd9e2e243267b7dcb664`. The exact executable SHA-256
values were:

- accepted pre-change baseline:
  `b0f2b76a9c3debb5f18a959643cd04f729e8edc95db5af009a2e21207c81e175`;
- compact-group candidate:
  `6693a55e1770295039a0251f56377100a5330ec793fe612dcdbba88aed12b695`;
- RocksDB 10.10.1 adapter:
  `e7510aac2bd6557c2a84418b43861fe17d8a06efff8e4959abc2f1cd74f6dba4`.

Every WSLDB run ended at transaction 4,096 with logical SHA-256
`cbd8ba5aeeb849af1dc5ba4c266622880cf776788942a74e7044c16a73d4b65d`.
Every RocksDB run independently verified all 4,096 expected values and exactly
4,096 entries.

### Causal result

| Median metric | Baseline | Compact group | Candidate result |
| :--- | ---: | ---: | ---: |
| Active throughput | 2,119.635 tx/s | 2,279.364 tx/s | 7.54% higher |
| WAL sync count | 355 | 347 | 2.25% fewer |
| Maximum RSS | 3,997,696 bytes | 4,030,464 bytes | 0.82% higher |
| WAL bytes | 802,816 | 496,959 | 38.10% fewer |
| Final storage | 811,008 bytes | 505,151 bytes | 37.71% smaller |

The candidate passed every frozen causal threshold. All five measured candidate
runs remained below the WAL-byte limit, and the small RSS difference remained
well inside the 10% allowance. The codec reduced bytes while improving median
throughput rather than trading one metric for the other.

### Fresh RocksDB result

| Median metric | WSLDB | RocksDB 10.10.1 | WSLDB relative result |
| :--- | ---: | ---: | ---: |
| Active throughput | 2,063.056 tx/s | 1,446.800 tx/s | 42.59% higher |
| WAL sync count | 338 | 495 | 31.72% fewer |
| Maximum RSS | 4,096,000 bytes | 10,747,904 bytes | 61.89% lower |
| WAL bytes | 496,288 | 493,364 | 0.59% more |
| Final storage | 504,480 bytes | 556,563 bytes | 9.36% smaller |

WSLDB won all five fresh throughput pairs by 49.30%, 47.44%, 28.81%,
42.59%, and 36.79%. It passed the pair, throughput, sync, RSS, and final-storage
gates. This closes the previously measured 45.71% final-storage deficit on this
specific concurrent strict-durability profile. WAL bytes are now effectively
at parity in this sample rather than 62.72% larger.

The complete all-feature suite passed 69 unit tests and every integration,
crash, I/O-failure, maintenance, refresh, fixture, and multi-process test,
followed by three documentation tests, all-target all-feature check, Clippy
with warnings denied, formatting, and diff check. Focused coverage validates
every possible truncation of a representative group, semantic summary
corruption, canonical and overflowing varints, old fixture readability, and
old-or-complete-group crash recovery. An additional short release corruption
sample mutated 200 main-file and 200 WAL cases without accepting an unknown
state; this is not substituted for the existing 100,000-case release evidence.

Raw causal output is retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-compact-group-wal`;
fresh comparison output is retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-compact-group-wal-rocksdb`.
Disposable per-run databases were moved to the system Trash after their sizes
and verification results were recorded.
