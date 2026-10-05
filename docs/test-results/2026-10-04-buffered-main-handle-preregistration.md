# Buffered main-handle reuse preregistration

Date: 2026-10-04. This document was committed before the candidate
implementation or its measurements.

## Observation

The current symmetric warm-cache W5/W6 smoke uses 100,000 entries, 100-byte
values, synchronous WAL-backed commits, a 64 MiB WSLDB write buffer, one
warm-up, and three measured five-second processes per engine and distribution.
WSLDB reached median active throughput of 721.705 operations/s on W5 and
734.102 operations/s on W6. Freshly rebuilt RocksDB 11.8.1 reached 801.750 and
794.266 operations/s respectively. WSLDB therefore remains 10.0% behind on W5
and 7.6% behind on W6.

A separate ten-second `sample(1)` capture over the W5 process attributed 344
of 6,824 main-thread samples to opening or closing the main database path from
the commit path. WAL synchronization remained the dominant cost. The opened
main handle is used only to confirm that the physical checkpoint still matches
the buffered generation; the storage engine already owns a handle for that
same file.

Raw comparison evidence is retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-current-sustained-smoke`.
The profile is retained at `/tmp/wsldb-sustained.sample.txt`. The baseline is
commit `b312797`, benchmark SHA-256
`584140f31ebd949eab3a0ac28673cabcdd04f3bfa080eee41c6d69d7cac34ed1`.

## Candidate

For buffered commits only, use the engine's existing main-file handle after
verifying under the existing cross-process commit lock that its Unix device
and inode still match the current database path. A mismatch remains a
`WriteConflict`. The candidate must retain the metadata-head comparison, WAL
identity and length checks, synchronous WAL publication, and all existing
fault behavior.

This changes no public API and no persisted bytes. Non-Unix targets retain the
current fresh-open path because they do not expose the required file identity
contract.

## Acceptance gate

The candidate is accepted only if all of the following hold:

- the existing current-file and stale-handle tests prove that an atomically
  replaced main file is rejected;
- the operation-limited uniform and Zipf adapter-conformance run still reaches
  the same operation count, entry count, and logical SHA-256 on both engines;
- relative to the fresh baseline/RocksDB ratios above, the candidate improves
  the WSLDB/RocksDB median active-throughput ratio by at least 3% on both W5
  and W6 in the same one-warm-up, three-measured-process smoke configuration;
- median point-read and mutation p99 do not regress by more than 5% relative
  to the simultaneous RocksDB control in either distribution;
- median maximum RSS and logical storage do not regress by more than 1%;
- formatting, all-target/all-feature checks and tests, Clippy with warnings
  denied, crash/fault/corruption coverage, and `git diff --check` pass.

Five-second measurements remain diagnostics and cannot satisfy the separate
long-duration production gate. If any acceptance condition fails, the
candidate code will be removed and the negative result retained.

## Result

Rejected. The candidate passed its focused identity, compaction-race,
multiprocess, refresh, group-commit, check, Clippy, and operation-limited
adapter-conformance checks. Its absolute median throughput increased from
721.705 to 755.011 operations/s on W5 (+4.62%) and from 734.102 to 770.496
operations/s on W6 (+4.96%). The simultaneous RocksDB medians also increased,
however, from 801.750 to 815.629 operations/s on W5 and from 794.266 to
824.051 operations/s on W6.

The authoritative normalized WSLDB/RocksDB ratio therefore improved by only
2.835% on W5 and 1.164% on W6. Both were below the required 3%, and the W5
point-read p99 ratio also worsened by more than its 5% guardrail. The candidate
implementation and its test were removed; buffered commits retain their
fresh-open validation path.

Candidate binary SHA-256:
`0ebc8b9b2ce8ecb90051e1b4c1d14975b3f8ddc3d6abe060835853833469470e`.
The freshly compiled RocksDB adapter SHA-256 was
`492fc9798912fcde5ea4ad26df9c30fffba67c89a3d6e3ec854f7e6c1079d331`.
Raw candidate evidence is retained at:

- `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-buffered-main-handle-conformance`
- `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-buffered-main-handle-smoke`
