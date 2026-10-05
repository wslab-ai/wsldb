# Compound buffered-preflight preregistration

Date: 2026-10-04. This acceptance contract is committed before candidate
implementation or measurement.

## Observation

Exact phase instrumentation attributes 5.825% of a clean short W6 buffered
commit interval to preflight and append-boundary validation. A previous
non-empty-WAL main-validation candidate improved median W6 active throughput
by 2.70%, won every paired process, and improved mutation tails, but was
removed because it missed its frozen 3% threshold.

The W6 write transactions contain one mutation. Point-index validation still
allocates an ordered-presence vector and a 64-element vector-of-vectors before
locking the one affected shard.

## Candidate

This compound candidate combines two disjoint preflight changes:

1. Restore the measured non-empty-WAL main-validation fast path. Under the
   cross-process commit lock, a non-empty current WAL excludes an independent
   main-file change: direct commits and compaction reject the WAL, external
   buffered commits change its exact append boundary, snapshot publication
   changes its file identity, and an in-process checkpoint serializes with the
   commit and replaces the storage generation. Empty-WAL generations retain a
   fresh main-path open and metadata-head validation.
2. Validate a one-mutation transaction directly against its single point-index
   shard. The general multi-mutation path and its deterministic shard grouping
   remain unchanged.

WAL identity and length checks, authoritative append-boundary validation,
transaction and entry-count checks, synchronous durability, recovery, and
post-sync publication remain unchanged. No public API or persisted byte change
is permitted.

## Frozen acceptance gate

The low-heat causal A/B uses separately built baseline and candidate binaries
against clones of the retained one-million-key W6 fixture: incompressible
100-byte values, Zipf 1.2, a 256 MiB page cache, and a 64 MiB durable buffer.
Each variant gets one unreported warm-up and three measured two-second fresh
processes, with variant order alternating by pair.

The candidate is accepted only if:

- median active throughput improves by at least 3%;
- median mean WAL preflight time improves by at least 15%;
- median overwrite and delete/recreate p99 regress by no more than 5%;
- median maximum RSS and final logical storage regress by no more than 1%;
- a separate 1,000-operation run preserves operation count, entry count,
  logical hash, main-file SHA-256, and WAL SHA-256;
- focused empty-WAL, non-empty-WAL, stale-writer, WAL replacement, checkpoint,
  compaction, refresh, group-commit, crash, and I/O-failure tests pass;
- formatting, all-target all-feature check, Clippy with warnings denied, and
  `git diff --check` pass.

The short run establishes only the causal candidate. If it passes, the frozen
one-million-key long W6 comparison still has to be repeated before claiming
RocksDB parity. A failed candidate and candidate-only tests will be removed;
the negative result will remain documented.

## Result

Accepted at source commit
`8741835fa46be2bdf58312b4fd8deb6310394a47`. The baseline binary SHA-256 was
`89f3043e9cd91f8da96136f5cd75856316320dd28091a559841edfa267c4a47a`; the
final candidate binary SHA-256 was
`1027dceed2ded19c9df2d0d6473ac83e494289d7f745f69cfb9ce54f60aeeefd`.

The first candidate exposed an omitted failure state: injected failure while
clearing a published checkpoint's WAL left an old storage generation with a
non-empty WAL and an advanced main file. The complete fault suite correctly
rejected a stale write. The implementation now marks every checkpoint metadata
publication as potentially advancing main before it begins. Such a generation
must reopen and validate main before any later fast-path commit; successful
validation clears the marker. The unsafe binary and raw diagnostic remain in
the evidence directory but are not part of this result.

Three alternating measured pairs of the corrected candidate produced these
medians:

| Metric | Baseline | Candidate | Candidate change |
| --- | ---: | ---: | ---: |
| Active throughput | 750.593 ops/s | 799.501 ops/s | +6.52% |
| Mean WAL preflight | 282.678 us | 86.690 us | -69.33% |
| Overwrite p99 | 11.032 ms | 10.246 ms | -7.13% |
| Delete/recreate p99 | 18.256 ms | 17.387 ms | -4.76% |
| Maximum RSS | 142,540,800 B | 142,442,496 B | -0.07% |
| Logical storage | 122,571,927 B | 122,571,927 B | 0.00% |

The candidate won active throughput in every measured pair. A separate
1,000-operation run preserved one million entries, logical SHA-256
`71b74312948753c324303dcdae124137456318a2baad7d8278cf225a03163e09`, main
file SHA-256
`1c1c871ebda3460aeaf6775f0973234493bf685d7a27870b10df77433833a41b`, and
WAL SHA-256
`7ab9004581b65ba4233741ad0224134d5e835bc8374d112f92d1473cc2c802d8`
between variants.

The complete all-feature unit, integration, multiprocess, crash, injected-I/O,
maintenance-race, and documentation suite passed after the safety correction.
All-target all-feature check, Clippy with warnings denied, formatting, and diff
validation also passed. Raw final logs and fixtures are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-compound-buffered-preflight-safe-ab`.

This result accepts the implementation but does not claim RocksDB parity. The
long frozen W6 comparison remains outstanding and should run only when a long
thermal load is acceptable.
