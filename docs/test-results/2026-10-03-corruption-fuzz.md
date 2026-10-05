# APFS corruption-fuzz evidence — 2026-10-03

## Subject

- Commit: `2aecf6d3bfa88f62fdf76bae61712bf5fc315f6c`
- Host: Apple arm64, macOS 26.6.2 (25G83), APFS on internal SSD
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Repository state before the run: clean

## Command

```text
WSLDB_RUN_CORRUPTION_FUZZ=1 \
WSLDB_CORRUPTION_FUZZ_ITERATIONS=100000 \
  cargo test --release --test corruption_fuzz -- --nocapture
```

Two deterministic tests ran concurrently. The main-file test copied a database
with two committed roots and applied 100,000 truncations or one-to-five bit
mutations to the 53,248-byte file. The write-ahead test copied a page-tree
checkpoint plus a consolidated snapshot record and two later transaction
records, then applied another 100,000 truncations or one-to-five bit mutations
to the 28,625-byte WAL.

Every successfully scanned main file had to equal one of its two complete
committed roots. Every successfully scanned WAL case had to equal the base tree,
the snapshot, the snapshot plus the first tail record, or the complete latest
state. A typed open or read failure was also acceptable. Any silently accepted
state outside those exact sets failed the run.

## Defect found and corrected

An earlier run found a silently accepted unknown state at main-file iteration
33,059. That deterministic mutation truncated an existing database to zero
bytes. Open treated it as a new database and initialized it, losing the two
expected committed entries from the observable state.

Opening and creating a database are now distinguished with an atomic
create-new attempt. Only the call that created a previously absent file may
initialize it; an existing zero-length file is a typed corruption error. A
focused regression test protects this invariant. The complete result below is
a clean rerun of the corrected commit, not a continuation of the failing run.

## Result

- Status: passed
- Main-file mutations: 100,000
- WAL mutations: 100,000
- Total mutations: 200,000
- Elapsed test time: 1,351.99 seconds
- Panics, hangs, or unknown accepted logical states: zero
- Peak process footprint reported by `/usr/bin/time -l`: 4,669,848 bytes
- Raw local evidence: `wsldb-benchmark-evidence/2026-10-03-corruption-fuzz-100k-apfs-fixed-clean`

This satisfies the finite corruption-mutation count on APFS. It is not evidence
for Linux, Windows, other filesystems, arbitrary corruption patterns outside the
deterministic corpus, or real power loss.
