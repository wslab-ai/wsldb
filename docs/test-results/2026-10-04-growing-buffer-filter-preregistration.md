# Growing write-buffer filter preregistration

Date: 2026-10-04. This contract is committed before implementation or
measurement.

## Observation

The concurrent RocksDB group diagnostic configured a 1 GiB durable buffer but
inserted only 4,096 unique keys. WSLDB won every throughput pair with 42.24%
higher median throughput and 34.95% fewer WAL syncs, yet failed the complete
gate because median maximum RSS was 25,001,984 bytes versus RocksDB's
10,780,672 bytes.

`new_buffer_key_filter` currently converts the configured byte limit into an
estimated maximum key count and allocates the complete 10-bit-per-key filter
when the database opens. A 1 GiB limit therefore reserves about 20 MiB before
the actual key population is known.

## Candidate

Replace only the write-buffer key filter with a geometrically growing filter.
The immutable-checkpoint membership filter remains fixed-size.

- Start with capacity for at most 65,536 unique buffered keys, or the estimated
  generation maximum when that is smaller.
- Before WAL synchronization, if a commit would exceed capacity, allocate a
  filter with doubled capacity until it fits and rebuild it from the exact
  ordered key map.
- Insert filter bits after durable publication without allocation. Failed
  pre-sync commits may leave extra filter capacity or false-positive bits, but
  never a false negative or visible mutation.
- Count only keys not already present in the current buffer, so hot-key updates
  do not cause growth.
- Continue geometric growth if the byte-based estimate is exceeded; the
  estimate is a sizing hint, not a correctness limit.
- Keep WAL encoding, transaction ordering, checkpoints, public API, and
  persisted bytes unchanged.

This uses the dynamic-sizing motivation of Almeida et al.'s
[Scalable Bloom Filters](https://doi.org/10.1016/j.ipl.2006.10.007), but chooses
geometric rebuild instead of layered queries because WSLDB already owns an
exact ordered key map. The implementation is original and must be judged by
WSLDB's own evidence.

## Frozen acceptance gate

Correctness requires focused tests for no false negatives before and after
multiple growth steps, duplicate-key updates not triggering growth, allocation
before the durable boundary, replay, snapshots, transaction groups, and
failure recovery. The complete all-feature unit, integration, crash,
I/O-failure, maintenance, and multi-process suite must pass, followed by
formatting, all-target check, Clippy with warnings denied, and diff check.

Performance uses the exact baseline binary and raw measurements from the
concurrent RocksDB group diagnostic. Run one unreported warm-up and five
alternating baseline/candidate pairs with 4,096 transactions and 16 callers.
The candidate must:

- retain the exact final logical SHA-256 and 802,816 WAL bytes;
- reduce median maximum RSS by at least 40%;
- use no more than RocksDB's previously measured 10,780,672-byte median RSS;
- regress median throughput by no more than 3%;
- use no more than 110% of the baseline median WAL sync count;
- show no new failure, panic, hang, checkpoint, or background drain.

If the causal gate passes, repeat five alternating candidate/RocksDB pairs
under the already frozen concurrent comparison. The earlier result is closed
only if the candidate again wins at least four pairs, leads median throughput
by at least 10%, uses no more than 110% of RocksDB's median sync count, and no
more median maximum RSS.

Failure removes the candidate rather than changing a threshold after
measurement. Passing remains a narrow APFS concurrency result, not general or
pinned-version superiority.

## Result

Rejected and removed. The candidate implementation was commit
`1bdf745cb4331cec8f9f05156b12a6466b6408ba`; removal was commit
`6af38451c145fbad49799d7c3b5ef0d7280ad165`.

The exact executable SHA-256 values were:

- baseline: `202d2fb5c21e3c19dd1f158c8983904fac284641103e3c4dfb2f6a09665d32b8`;
- candidate: `05bbb0761c9f8c8c6d95c5486025c81716a9090c78cbf9f40cb3685032d0c59f`.

Every measured process ended at transaction 4,096 with logical SHA-256
`cbd8ba5aeeb849af1dc5ba4c266622880cf776788942a74e7044c16a73d4b65d`,
802,816 WAL bytes, and 811,008 total bytes. No checkpoint or background drain
occurred.

| Median metric | Baseline | Candidate | Candidate result |
| :--- | ---: | ---: | ---: |
| Active throughput | 1,811.044 tx/s | 1,717.399 tx/s | 5.17% lower |
| WAL sync count | 351 | 346 | 1.42% fewer |
| Maximum RSS | 24,920,064 bytes | 4,014,080 bytes | 83.89% lower |

The candidate reduced median RSS below RocksDB's previously measured
10,780,672 bytes by 62.77%, decisively passing both memory gates. It won only
the fifth throughput pair; pair changes were -8.77%, -9.34%, -3.36%, -9.89%,
and +8.01%. Its 5.17% median regression exceeded the frozen 3% allowance, so
the causal gate failed and the candidate/RocksDB rerun was not performed.

Before measurement, the candidate passed 63 unit tests, every integration,
crash, I/O-failure, maintenance, and multi-process test, three documentation
tests, all-target all-feature check, Clippy with warnings denied, formatting,
and diff check. Correctness was not the rejection reason.

The result validates the diagnosis that eager maximum sizing accounts for the
concurrent profile's RSS, but this geometric-rebuild implementation does not
meet the joint performance contract. Its source and focused tests were removed
completely rather than retaining an unaccepted optimization or changing the
threshold after observing results. A future memory candidate needs to avoid
additional per-commit filter preparation on the durable grouping path and must
receive a new preregistration.

Raw output and both exact binaries remain under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-growing-buffer-filter`.
Disposable per-run databases were moved to the system Trash after their sizes
and verification results were recorded.
