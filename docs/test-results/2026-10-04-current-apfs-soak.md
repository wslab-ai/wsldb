# Current APFS model-soak evidence

Date: 2026-10-04.

## Subject

- Commit: `a932f9ae02b4a0a40f5a0a25a094b9c22c80e461`.
- Host: Apple M1 Max, macOS 26.6.2 (25G83), APFS on the internal SSD.
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`.
- Repository state before the run: clean.
- Subject path: the current direct-commit tree, page store, reopen, and atomic
  compaction implementation. This workload does not exercise the buffered WAL.

## Command

```text
WSLDB_RUN_SOAK=1 cargo test --release --test soak \
  model_based_ten_million_operation_soak -- --exact --nocapture
```

The deterministic workload executed 10,000,000 mixed inserts, overwrites, and
deletes in 1,000-operation durable transactions over a 100,000-key space. Every
100,000 operations it checked entry counts, compared 1,000 deterministic point
reads and 100 bounded or open-ended range reads against an independent
`BTreeMap`, closed the database, optionally compacted it, and reopened the
published generation. Compaction was triggered after retired bytes reached
512 MiB. The test ended with a complete ordered comparison against the model.

## Result

- Status: passed.
- Operations: 10,000,000.
- Durable transactions: 10,000 workload commits.
- Checkpoints with point/range comparison and reopen: 100.
- Sampled point reads: 100,000.
- Model-compared range reads: 10,000.
- Verified atomic compactions: 33.
- Final transaction ID: 10,033.
- Largest observed pre-compaction generation: 770 MiB.
- Compacted generation size: 5 MiB at every rotation.
- Final file state: 499 MiB total, 5 MiB live, 493 MiB retired.
- Test elapsed time: 415.62 seconds.
- Whole command time: 420.17 seconds.
- Maximum resident set size: 396,902,400 bytes.
- Final full ordered model comparison: exact match.
- Failures, panics, hangs, and leftover soak files: zero.

The complete log has SHA-256
`6c11638b82f932bc91326494e75736cf76a1f1c6b5f3af6e09f773a1e0ca125d`.
The manifest and raw log are retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-current-apfs-model-soak`.

This closes the finite current-code direct-storage model gate on APFS. It does
not replace buffered-WAL recovery testing, real power-loss testing, the open
current-format Linux rerun, or evidence on other filesystems.
