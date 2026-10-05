# Sustained comparison runner contract

Source commit under test: `bab4294` (`fix: isolate sustained benchmark
environment`).

## Risk

The W5/W6 runner previously used `env` without removing inherited benchmark
variables. An operator could therefore record `operation_limit=0` while an
ambient `WSLDB_BENCH_OPERATION_LIMIT` silently shortened both adapters, or
enable a setup drain or write buffer that was absent from the manifest.
RocksDB WAL statistics were enabled by variable presence but were neither
validated nor recorded. The runner also accepted an empty or duplicated
distribution list and recorded less host/filesystem context than the short
comparison runner.

The cache-page arithmetic was evaluated before validating `CACHE_BYTES` as a
positive decimal integer. This was not a database-input path, but it weakened
the fail-closed shell boundary for release evidence.

## Contract

The runner now:

- validates every integer before using it in arithmetic;
- requires one or two unique `uniform`/`zipf` distributions;
- accepts RocksDB WAL statistics only as an unset variable or the exact value
  `1`, then records the resolved mode;
- removes inherited operation-limit and optional-statistics variables before
  invoking RocksDB, then adds only the recorded values;
- removes inherited operation-limit and load-drain controls before invoking
  WSLDB, uses a direct tree for setup, and passes the recorded write-buffer
  setting only to the measured process;
- records the fixed seed, scratch parent, filesystem capacity, mount table,
  CPU, memory, storage, kernel, Rust, and Cargo context.

The command log retains the exact `env -u` invocation and all explicit child
settings. Release mode continues to require clean pinned source and binary
identity, while every individual output still records
`release_gate_complete=no` until the whole external matrix is assembled.

## Verification

The shell contract uses executable stubs rather than either database. It starts
with conflicting ambient values (`operation_limit=9`, load drain enabled, and
write buffer `999`), requests the recorded values `1`, disabled drain, and
write buffer `0`, and makes each stub fail unless it sees the sanitized
environment. The same run verifies byte-equivalent completion fields and the
manifest boundaries. Separate cases require failure for an empty distribution
list, a duplicate distribution, and `WSLDB_ROCKSDB_WAL_STATS=0`.

```text
bash -n benchmarks/run-sustained-comparison.sh \
  benchmarks/benchmark-contract.test.sh
benchmarks/benchmark-contract.test.sh
  benchmark contract tests passed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

git diff --check
  passed
```

No WSLDB or RocksDB workload, cache reset, reboot, shutdown, sleep, power
operation, raw-device access, or physical power-loss test was performed. This
is evidence for runner integrity only; the pinned repeated W5/W6, cold-cache,
Linux/ext4, and physical power-loss release gates remain open.
