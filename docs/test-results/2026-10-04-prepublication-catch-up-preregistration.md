# Pre-publication catch-up preregistration

Date: 2026-10-04. This contract is committed after the chunked-maintenance
pause failure and before implementing or measuring this candidate.

## Causal diagnosis

Bounded 4 MiB maintenance chunks reduced median RSS by 59.49% and compaction
wall time by 5.93%, but publication-lock pause regressed by 69.58%. The
replacement rebuild is already concurrent; the pause comes from applying and
syncing every WAL mutation accumulated during that rebuild while writers are
blocked.

The delta-catch-up recovery proof does not require that work to begin under the
final lock. Until rename, the replacement is non-authoritative: a crash leaves
the original main file plus durable WAL intact. This permits one optimistic
catch-up round before final publication, analogous to the log-assisted replica
construction described by O'Toole, Nettles, and Gifford:
<https://www.cs.cmu.edu/afs/cs/project/venari/www/sosp93.html>.

## Frozen protocol

Baseline source commit:
`e1e32b25febf75849ab69ce8209a6c855805d6ea`.
The candidate includes the rejected candidate's single traversal and fixed
4 MiB/32,768-entry maintenance chunks, plus this pause reduction:

1. Build and verify the dense replacement at captured transaction `T` while
   holding the existing main-file lease.
2. Briefly acquire the commit and sidecar locks, refresh a valid external WAL
   tail, revalidate physical checkpoint and WAL identity, and capture logical
   head `C` plus the net changes after `T`. Release both locks without changing
   the authoritative files.
3. If `C > T`, apply that delta to the replacement at transaction `C + 1` and
   durably sync it while foreground WAL commits continue.
4. Acquire the final publication locks and refresh again. If current head `D`
   is newer than `C`, apply only changes after `C` to the replacement at
   transaction `D + 1`. Otherwise publish the already caught-up `C + 1`
   replacement. Rename, directory sync, and WAL retirement remain unchanged.
5. Fully verify the reopened generation after releasing publication locks.

The first catch-up may leave retired pages inside the replacement, but its
space remains bounded by changed paths and is measured. No format, public API,
commit durability, or recovery rule changes.

## Frozen acceptance gates

Run three new alternating fresh-process pairs on clones of the same 100,000-key
fixture. Accept only if the combined candidate satisfies every gate:

- median maximum RSS is at least 25% below baseline;
- median compaction wall time regresses by no more than 15%;
- median publication-lock pause and concurrent commit p99 regress by no more
  than 25%;
- every run preserves exact logical verification, empties the WAL, reports
  only the expected stale-generation conflict, and reduces storage by at least
  5%;
- a deterministic two-wave race proves that commits before and during the
  optimistic catch-up both survive with exact transaction ordering;
- crash and injected-I/O matrices cover optimistic and final catch-up syncs,
  rename, and WAL retirement, always recovering the original main plus WAL or
  the complete new generation;
- overlay, maximum-key, empty-physical-tree, backup, named compaction,
  freeze-and-swap, and existing maintenance suites pass;
- a deterministic 1,000-operation no-maintenance control is byte-identical to
  baseline;
- formatting, complete all-target all-feature tests, focused release tests,
  all-target check, Clippy with warnings denied, and `git diff --check` pass.

A passing result establishes bounded-memory online maintenance with bounded
final pause for this profile. It does not establish automatic scheduling or
general RocksDB superiority.

## Result

Rejected. No source commit contains this candidate.

The optimistic round preserved both deterministic commit waves and reduced the
combined candidate's publication work relative to the preceding chunk-only
attempt. The complete frozen comparison still failed both foreground-latency
guards.

| Median | Baseline | Combined candidate | Change | Gate |
| --- | ---: | ---: | ---: | ---: |
| Maximum RSS | 25,624,576 B | 10,452,992 B | -59.21% | at most -25% |
| Compaction wall time | 2.027 s | 2.269 s | +11.91% | at most +15% |
| Publication-lock pause | 39.717 ms | 53.009 ms | +33.47% | at most +25% |
| Concurrent commit p99 | 11.672 ms | 16.455 ms | +40.98% | at most +25% |

Every run verified successfully, retired the WAL, reported only the expected
generation conflict, and reduced storage by more than 96%. Those correctness
results do not override the failed pause and p99 gates. The implementation is
removed rather than weakening thresholds after measurement.

Raw binaries and logs are retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-prepublication-catch-up`.
The evidence confirms that bounded-memory reconstruction is feasible, but its
I/O scheduling must be redesigned before adoption; additional catch-up rounds
alone do not protect foreground tails.
