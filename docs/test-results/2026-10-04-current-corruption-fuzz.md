# Current APFS corruption-fuzz evidence

Date: 2026-10-04.

## Subject

- Commit: `5650ee172bc07faef12fadd9b2823e133592657d`.
- Host: Apple M1 Max, macOS 26.6.2 (25G83), APFS on the internal SSD.
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`.
- Repository state before the counted run: clean.
- Persisted format: current unreleased main-file format and prefix-encoded WAL.

## Command

```text
WSLDB_RUN_CORRUPTION_FUZZ=1 \
WSLDB_CORRUPTION_FUZZ_ITERATIONS=100000 \
  cargo test --release --test corruption_fuzz -- --nocapture
```

The two deterministic tests ran concurrently. One copied and mutated a valid
main file containing two committed roots. The other copied and mutated a valid
WAL containing a consolidated snapshot plus an ordered transaction tail. Each
iteration either truncated its target or flipped one to five selected bits.

Every successfully opened main file had to equal one of its complete committed
roots. Every successfully replayed WAL had to equal one of the explicitly
allowed complete prefixes. A typed corruption or I/O error was acceptable;
panic, hang, silently accepted unknown state, or mixed transaction state failed
the test.

## Result

- Status: passed.
- Main-file mutations: 100,000.
- Prefix-encoded WAL mutations: 100,000.
- Total mutations: 200,000.
- Test elapsed time: 3,864.95 seconds.
- Whole command time: 3,865.01 seconds.
- Maximum resident set size: 21,561,344 bytes.
- Panics, hangs, unknown accepted states, or leftover fuzz files: zero.

An initial run was intentionally interrupted at approximately 35% when only
1.5 GiB remained on the host filesystem. It is retained with an
`interrupted-low-space` name and is not counted. Stale WSLDB benchmark temp
files and reproducible RocksDB object files were removed, restoring 5.5 GiB;
the complete counted rerun then held that free-space level throughout.

Raw manifest, complete progress log, process result, resource report, and the
interrupted diagnostic are retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-current-corruption-fuzz`.

This closes the finite current-format corruption gate on APFS. It does not
replace the open current-format Linux rerun, other filesystems, arbitrary
corruption outside the deterministic corpus, or real power-loss testing.
