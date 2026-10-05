# Shared ordered version-index handoff

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation

Streaming WAL snapshot publication reduced the frozen W2 median peak RSS from
316,211,200 to 203,669,504 bytes. The handoff still clones every node in the
ordered `BTreeMap`, even though the old and new generations initially contain
the same key-to-version mapping and both already share key allocations and the
append-only version arena.

## Candidate

The first handoff of a mutable ordered map will seal that map into one shared
immutable `Arc` instead of cloning its nodes. The old generation and the new
generation will reference the same sealed map. Later commits write only to a
new mutable ordered overlay; point lookup checks the overlay before the sealed
base, and scans merge both ordered sources with the overlay winning duplicate
keys.

A later handoff with a non-empty overlay will materialize one new sealed map.
This keeps the representation at two levels and prevents an unbounded chain of
indexes. It moves an existing full-map copy away from the common first
handoff, but does not weaken the worst-case behavior of repeated explicit
flushes. Transaction histories remain in the shared append-only arena, so old
readers continue selecting their visible version by transaction ID. The WAL,
page, metadata, and recovery encodings do not change.

## Frozen acceptance gate

The causal A/B uses one million unique 16-byte keys, incompressible 100-byte
values, 10,000 mutations per synchronous commit, a 64 MiB durable write
buffer, and 65,536 decoded cache pages. The baseline is storage commit
`f2a792c`; one warm-up and five measured fresh-file processes per variant are
alternated, and every process explicitly drains pending work.

The candidate is accepted only if median maximum RSS decreases by at least
8%, median active throughput does not regress by more than 2%, and median
active-plus-drain time does not regress by more than 3%. Median maximum commit
latency may not exceed 25 milliseconds. Final logical, main-file, and WAL
hashes and lengths must match the baseline exactly, and checkpoint failures
must remain zero.

Focused tests must cover base-only keys, overlay-only keys, overwritten keys,
tombstones, ordered scans and ranges across both sources, initialized point
indexes, old readers, later commits, a second handoff that flattens the
overlay, reopen, corruption, short writes, and every existing crash boundary.
Formatting, all-target all-feature tests, Clippy with warnings denied, check,
diff-check, and the 700-line source limit must pass. Thresholds will not change
after candidate results are observed.

## Result: rejected and removed

The candidate failed both primary gates and was removed completely. One
warm-up per variant preceded five alternating measured fresh-file processes.
The baseline was the accepted streaming-snapshot binary from `f2a792c`.

| Median metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Maximum RSS | 203,390,976 B | 204,931,072 B | +0.76% |
| Active W2 throughput | 608,013.820 ops/s | 579,254.311 ops/s | -4.73% |
| Active load time | 1.645 s | 1.726 s | +4.97% |
| Active plus drain time | 1.845 s | 1.915 s | +3.81% |
| Standalone drain | 204.692 ms | 188.200 ms | -8.06% |
| Maximum commit latency | 24.473 ms | 20.148 ms | -17.68% |

Sharing the first immutable map removed one explicit node clone, but it did not
reduce the process high-water mark. The merged base/overlay iterator added a
branch and two-source ordering work to every checkpoint and verification pass;
the observed throughput and active-time regressions exceeded their frozen
limits. The drain improvement and lower commit tail were insufficient to
accept that steady-state read and checkpoint cost.

Every measured process still reported zero checkpoint failures and produced
the same logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`,
main-file hash
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`,
WAL hash
`21ba0081c157f7ca7a34adc93fd26e72c8ea3623abff1261d0be43cc9081eedd`,
and main/WAL lengths of 65,486,848 and 57,960,068 bytes. Correctness was not
the rejection reason.

The baseline and rejected-candidate benchmark SHA-256 values were respectively
`49fbecf164e71f3fd37bda11c6698d48c00e6ad4ad76ed03e44959398c03d7dc`
and
`5d49aad610d4f694fc087fb55ab2d5299233149687b828ce38b4662c89ed4de3`.
Raw outputs, resource reports, hashes, sizes, binaries, and retained databases
are outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-shared-version-index-w2-ab`.
