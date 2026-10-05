# Endurance and corruption testing

Long tests are opt-in so the normal local release checklist stays fast. The
model-based soak defaults to ten million deterministic insert, overwrite, read,
delete, commit, verification, and reopen operations against an independent
`BTreeMap` model:

```sh
WSLDB_RUN_SOAK=1 cargo test --release --test soak -- --nocapture
```

The harness reports physical, live, and retired bytes at every 100,000-operation
checkpoint. By default it atomically compacts the active path after retired
bytes reach 512 MiB, exercising bounded-space operation and stale-handle
protection. Set `WSLDB_SOAK_COMPACT_BYTES` to another byte threshold when
testing different storage budgets.

`WSLDB_SOAK_OPERATIONS` can lower the count for harness smoke checks. Release
evidence must use the default ten million operations and record the commit,
platform, filesystem, elapsed time, and final operational stats.

Recorded runs are stored under `docs/test-results/`.

The arbitrary-byte harness builds two valid snapshots, repeatedly copies the
file, flips random bits or truncates it, then opens and fully verifies the
result. A verified file must equal one of the two committed logical states;
otherwise the operation must return a typed error.

```sh
WSLDB_RUN_CORRUPTION_FUZZ=1 \
  cargo test --release --test corruption_fuzz -- --nocapture
```

`WSLDB_CORRUPTION_FUZZ_ITERATIONS` controls the number of mutations performed by
each of the two targets. The finite production gate is 100,000 main-file and
100,000 WAL mutations on each supported platform:

```sh
WSLDB_RUN_CORRUPTION_FUZZ=1 \
WSLDB_CORRUPTION_FUZZ_ITERATIONS=100000 \
  cargo test --release --test corruption_fuzz -- --nocapture
```

Retain the process logs, exact commit, filesystem, elapsed time, and final
mutation counts as release evidence.
