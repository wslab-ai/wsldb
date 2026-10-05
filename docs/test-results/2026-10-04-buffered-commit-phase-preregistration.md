# Buffered commit phase timing preregistration

Date: 2026-10-04. This document is committed before instrumentation or
measurement.

## Observation

Exact WAL timings show that a clean short W6 interval spent 94.038% of
successful commit time in stable-storage synchronization and 1.877% issuing
the WAL write. The remaining 4.085% still combines path locking, main/WAL
validation, entry-count lookup, encoding, and post-sync in-memory publication.
Several isolated fast paths were rejected because short filesystem variance
was larger than their effect.

## Instrumentation

Extend successful buffered-commit observability with non-overlapping timing
counters:

- `wal_preflights`: from storage commit entry through locks, file open,
  validation, presence lookup, and the last fallible in-memory check before
  WAL encoding, plus the authoritative append-boundary length validation
  immediately before the positioned write;
- `wal_encodes`: canonical transaction-record encoding before the write;
- existing `wal_writes`: the positioned write only;
- existing `wal_syncs`: `File::sync_all` only;
- `wal_publications`: work after durable append through in-memory visibility,
  point-index publication, and any synchronous checkpoint follow-up before
  commit completion.

Ordinary buffered commits, explicit groups, and automatically coalesced groups
use the same counters. Failed commits do not increment successful-operation
counters. Direct copy-on-write commits leave all WAL phase counters at zero.

## Validation and interpretation

- Each successful buffered WAL append increments every phase exactly once.
- Focused ordinary, explicit-group, automatic-group, and direct-mode tests
  validate counter parity.
- Public API compilation, formatting, all-target check, Clippy with warnings
  denied, and `git diff --check` must pass.
- A clean two-second W6 diagnostic may route the next optimization, but cannot
  establish a performance change. The largest repeatable non-sync phase is the
  next candidate; if no phase dominates or absolute sync time changes
  materially, no production fast path is accepted from the short run.

## Result

Instrumentation and focused validation passed at clean commit
`bb037a0f48dc66559c5727b462355a00ba23e43a`. The optimized benchmark SHA-256
was `89f3043e9cd91f8da96136f5cd75856316320dd28091a559841edfa267c4a47a`.
Direct commits reported zero operations for every WAL phase. Ordinary,
explicit-group, and automatically coalesced commits reported exactly one of
each phase per successful WAL append, including injected partial-write and
sync-failure coverage.

The clean two-second W6 diagnostic used the frozen one-million-key,
incompressible 100-byte-value, Zipf-1.2 profile, a 256 MiB decoded-page cache,
and a 64 MiB durable buffer. It completed 1,687 operations and 405 durable
commits. The short throughput of 841.740 operations/s is noise-sensitive and
is not a comparison result.

| Successful commit phase | Total | Mean/commit | Share of commit time |
| :--- | ---: | ---: | ---: |
| Preflight and boundary validation | 115.754 ms | 0.286 ms | 5.825% |
| WAL encoding | 19.934 ms | 0.049 ms | 1.003% |
| Positioned WAL write | 44.201 ms | 0.109 ms | 2.224% |
| Stable-storage synchronization | 1,788.788 ms | 4.417 ms | 90.011% |
| Post-sync publication | 10.168 ms | 0.025 ms | 0.512% |
| Timer/accounting residual | 8.450 ms | 0.021 ms | 0.425% |

The five measured phases account for 99.575% of the 1,987.295-millisecond
commit total. Synchronization remains dominant and varied materially from the
earlier clean two-second diagnostic. Preflight is nevertheless the only
repeatable non-sync phase large enough to justify the next bounded candidate;
encoding and publication are too small to motivate architectural changes in
isolation.

The final database retained exactly one million entries and verified logical
SHA-256
`e4132bab3d7a6847d26a9762425275ffe0b3db3590d1bd08621270b8f7ed2607`.
The main and consolidated WAL files are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-buffered-phases`
with SHA-256 values `1c1c871e...33a41b` and `308cddaa...22956`.
