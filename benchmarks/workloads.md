# Frozen benchmark workloads

These workloads are preregistered for the production-readiness comparison. Do
not change their distributions or success thresholds after viewing comparative
results; add a separately named workload instead.

## Common configuration

- Keys: the official RocksDB `db_bench` 16-byte `GenerateKeyFromInt`
  representation: an eight-byte big-endian integer followed by eight ASCII
  zero bytes. Both engines receive byte-identical keys.
- Values: deterministic incompressible payloads of 100 bytes and 4096 bytes.
- Dataset sizes: 1 million, 10 million, and a dataset at least four times RAM.
- Durability: every measured commit is synced for both engines.
- Cache states: report warm-cache and cold-cache runs separately. Cold r0 and
  r1–r5 each run in a fresh process after a recorded OS-cache reset.
- Repetitions: five measured runs after one discarded run; report every
  measured run and retain the discarded run in raw evidence.
- The warm W4 discarded run performs enough random ranges to touch an
  estimated five cache capacities (capped at 1 million ranges). The cold W4
  discarded run remains cold and is isolated like every measured cold run.
- Record throughput, p50, p95, p99, p99.9, CPU time, RSS, bytes read/written,
  final file size, and open-to-first-read latency.
- WSLDB load reports include logical mutation bytes, database data/metadata
  bytes written, their ratio, and live/retired bytes. The ratio measures the
  database file writes, not hidden filesystem or device amplification.

## W1 sequential load

Insert ascending keys in batches of 1, 100, and 10,000.

## W2 random load

Insert a fixed-seed permutation of the keyspace using the same batch sizes.

## W3 random point reads

Run 50% present and 50% absent uniformly random lookups with 1, 4, 16, and 64
threads. Readers use a committed snapshot.

## W4 ordered range scans

Choose uniform random starts and scan 10, 100, and 10,000 consecutive entries.

## W5 sustained updates

Maintain a fixed-size database with 80% point reads, 15% overwrites, and 5%
delete/recreate cycles for 30 minutes. A delete/recreate cycle durably deletes
the selected key and then durably recreates it, so it exercises deletion while
preserving the key distribution and entry count. Both adapters perform an exact
existence lookup inside the measured cycle and fail if the selected key is
missing; RocksDB's `Delete` status alone cannot prove existence. Report its
combined latency as one operation. Report latency over time so stalls cannot
be averaged away.
The warm-cache profile performs one complete ordered preload through each
engine's public iterator before starting the timer. The preload is outside the
measurement window and its exact entry count must match the configured dataset.

## W6 skewed updates

Use the same operation mix as W5 with a fixed Zipfian distribution and report
the configured exponent.

## W7 recovery and startup

Measure clean open, recovery after injected interruption, full verification,
backup, restore, and open-to-first-read latency.

The comparison uses the official RocksDB `db_bench` tool and records its commit,
complete options, compiler, filesystem, kernel, CPU, RAM, and storage device.

## WSLDB harness

The dependency-free WSLDB harness currently exercises W1, W3 at 1/4/16/64
reader threads, W4, and the clean startup portion of W7. It is opt-in so
`cargo test --all-targets` never starts a long benchmark accidentally:

```sh
WSLDB_RUN_BENCHMARK=1 cargo bench --bench wsldb -- 100000 100000 1000 4096
```

The positional arguments are entries, measured operations, durable commit batch
size, and decoded-page cache capacity. Output metrics are CSV on stdout; run
configuration, cache counters, and final file size are written to stderr. The
harness accepts these environment controls:

- `WSLDB_BENCH_DATABASE` selects a persistent benchmark database path.
- `WSLDB_BENCH_WORKLOAD` selects `all`, `load`, `random-load`, `reads`,
  `ranges`, `open`, `sustained`, or `skewed`.
- `WSLDB_BENCH_VALUE_BYTES` selects the value size.
- `WSLDB_BENCH_WRITE_BUFFER_BYTES` enables the durable write buffer and sets
  its checkpoint threshold; zero selects direct copy-on-write commits.
- `WSLDB_BENCH_THREADS` selects 1, 4, 16, or 64 read threads.
- `WSLDB_BENCH_RANGE` selects the number of entries returned by each range.
- `WSLDB_BENCH_DURATION_SECONDS` selects W5 duration (1800 by default).
- `WSLDB_BENCH_COMPACT_BYTES` selects the W5 retired-byte compaction trigger.
- `WSLDB_BENCH_MAINTENANCE_SECONDS` selects the W5 verification interval.
- `WSLDB_BENCH_ZIPF_EXPONENT` selects the W6 exponent (1.2 by default).
- `WSLDB_BENCH_OPERATION_LIMIT` stops W5/W6 after an exact positive operation
  count; it is intended for cross-engine semantic conformance, not timed
  throughput claims.
- `WSLDB_BENCH_MEASURED_RUNS` selects the positive number of W3/W4 measured
  repetitions and defaults to five.
- `WSLDB_BENCH_RUN_START` selects the first non-negative W3/W4/W7 run ID. A
  cold runner uses zero for its discarded warm-up and one through five for the
  measured processes.
- `WSLDB_BENCH_WARMUP=0` disables the in-process W3/W4 warm-up; the default is
  `1`. Disabling this flag does not evict the operating-system page cache and
  is valid cold-cache evidence only when an external recorded eviction step
  runs before that process.

W5 requires a database created by a separate `load` invocation at the same
`WSLDB_BENCH_DATABASE` path. It emits one-second read, overwrite, delete, and
combined windows, plus exact whole-run distributions with a `_total` suffix.
Each window records checkpoint starts, completions, failures, bytes, active
work at its boundaries, and an explicit activity flag. Cumulative checkpoint
totals survive the database-handle replacement performed after active
compaction. Before replacing that handle, the harness explicitly drains and
measures any pending checkpoint and fails if background checkpoint work remains
active. Verification, checkpoint drain, final drain, and compaction pauses are
separate metrics, so maintenance stalls are not hidden inside an average,
attributed to the next active window, or allowed to overlap compaction.

The `skewed` mode runs the same sustained mix with rejection-inversion sampling
from a finite Zipf distribution. It uses constant memory even for the 4× RAM
dataset and records the configured exponent with the benchmark environment.

## Reproducible RocksDB comparison

Build the official RocksDB `db_bench` executable from a pinned commit, then run:

```sh
benchmarks/run-comparison.sh /path/to/rocksdb/db_bench /new/evidence/directory
```

Release mode is the default. Before creating the evidence directory, the
runner requires clean WSLDB and RocksDB checkouts, RocksDB commit
`abeebd9630f11bd08c28b7bd43c7bdfc62050654` (the official `v11.8.1` tag), and
a `db_bench` binary that reports exactly version `11.8.1`. Set
`WSLDB_ROCKSDB_SOURCE` when the binary is outside its source checkout.
Release-mode W7 also requires the open adapter below and verifies its linked
RocksDB version. A mismatch fails before any timed workload starts.

Candidate investigation against a dirty checkout or another RocksDB build must
set `WSLDB_BENCHMARK_RELEASE_MODE=0`. The manifest then records
`source_contract_satisfied=no`; such output is diagnostic evidence and cannot
satisfy a release gate. Strict mode records `source_contract_satisfied=yes`
but still records `release_gate_complete=no`: one output is only a contribution
to the required dataset, cache-state, platform, and workload matrix. Strict
mode also builds WSLDB itself from the recorded clean checkout rather than
accepting `WSLDB_BENCH_BIN`.

`benchmarks/rocksdb_open_bench.cc` is a minimal adapter for the exact W7
open-to-first-read interval. Build it against the same RocksDB checkout and
static library as `db_bench`, including that build's platform link flags, then
set `WSLDB_ROCKSDB_OPEN_BENCH` to its executable path. Without the adapter the
runner retains the `db_bench` startup diagnostic but does not treat it as a
direct W7 comparison.

```sh
c++ -std=c++20 -O3 -I"$ROCKSDB/include" \
  benchmarks/rocksdb_open_bench.cc "$ROCKSDB/librocksdb.a" \
  $ROCKSDB_PLATFORM_LIBS -o /path/to/rocksdb_open_bench
```

`ROCKSDB_PLATFORM_LIBS` must contain the same compression, flags, threading,
and platform libraries used for that RocksDB build.

The runner alternates engine order for W1 and W2, performs one warm-up and five
measured runs, keeps compression disabled, enables synchronous commits and RocksDB WALs,
and uses equal key, value, and cache sizes. It captures the exact executed commands,
raw stdout/stderr, operating-system resource measurements, repository commits,
hardware/filesystem metadata, and final storage sizes. Scratch databases are
created under `TMPDIR` and removed only after their measurements are recorded.

The default invocation covers 1 million entries with both specified value sizes.
Use `WSLDB_COMPARISON_ENTRIES`, `WSLDB_COMPARISON_OPERATIONS`,
`WSLDB_COMPARISON_RANGE_OPERATIONS`, `WSLDB_COMPARISON_CACHE_BYTES`, and the
space-separated `WSLDB_COMPARISON_LOADS`, `WSLDB_COMPARISON_BATCHES`,
`WSLDB_COMPARISON_THREADS`, `WSLDB_COMPARISON_RANGES`, and
`WSLDB_COMPARISON_VALUES` controls to execute the other preregistered dataset
sizes without changing the workload code.

`WSLDB_COMPARISON_PHASES` selects any space-separated subset of `loads`,
`reads`, `ranges`, and `opens`. The default executes all four. Read, range, and
open phases share one freshly loaded database for each value size; an
opens-only run therefore measures the exact W7 interval without rerunning W1
through W4.

`WSLDB_COMPARISON_CACHE_STATE=cold` selects isolated cold-cache W3, W4, and W7
processes. It requires `WSLDB_COLD_CACHE_COMMAND` to name an executable. Before
every engine's discarded or measured process, the runner executes that program
as `COMMAND DATABASE LABEL`, captures its stdout and stderr, records the exact
invocation, and hashes the executable into the manifest. Engine order
alternates for cold pairs. WSLDB receives one measured run, no in-process
warm-up, and a fresh decoded-page cache in every process; RocksDB likewise
receives one benchmark repetition per process.

The reset program is platform-specific and deliberately not supplied by the
runner itself: APFS, ext4, XFS, virtual machines, and bare metal require
different privileges and proof. The repository provides fail-closed reference
helpers, `benchmarks/cache-reset-macos.sh` (`sync` plus Apple's `purge`) and
`benchmarks/cache-reset-linux.sh` (`sync` plus Linux `drop_caches`). Run the
appropriate helper with `--check-host` to validate platform support and with
`--check` to require ready non-interactive privilege before scheduling
evidence, then pass its absolute path as `WSLDB_COLD_CACHE_COMMAND`.

Both reference helpers affect the host's global file cache and must run only
on an isolated benchmark machine. They require root or previously authorized
non-interactive `sudo` that remains valid for the complete matrix; they never
prompt inside a timed runner. Exit status zero is necessary but not sufficient
release evidence. The result record must document the privilege setup and
independently verify that the tested main file, WSLDB sidecar where present,
and RocksDB files were evicted. The default `warm` state does not invoke either
helper.

`WSLDB_COMPARISON_WRITE_BUFFER_BYTES` selects WSLDB's durable write-buffer
threshold and defaults to zero, the direct COW path. Buffered comparisons count
both the main file and `.wal` sidecar in logical and allocated storage, and the
runner removes both before starting the next fresh process.

This runner currently covers W1, W2, W3, W4, and clean open-to-first-read
evidence from W7. W7 is directly comparable only when the exact adapter is
configured; the `db_bench` fallback includes a different interval. The runner
now implements the cold-cache W3/W4/W7 protocol, but current supported-platform
cold evidence is still required. W5, W6, and interrupted recovery remain
separate release gates. Smoke-test numbers are not release claims.

## Explicit group-commit diagnostic

The separate dependency-free harness compares individual buffered commits,
the explicit group API, and automatic concurrent `commit_batch` calls using
byte-identical keys and values. Every path executes the same number of logical
transactions and must finish with the same logical SHA-256. The group-size
argument is also the automatic-mode thread count. The harness reports elapsed
time, transaction throughput, WAL syncs, and WAL bytes; it is diagnostic
evidence and does not replace the frozen RocksDB workloads above.

```sh
WSLDB_RUN_GROUP_BENCH=1 \
WSLDB_GROUP_BENCH_DIRECTORY=/path/on/test/filesystem \
  cargo bench --bench group_commit -- 4096 16
```

`WSLDB_GROUP_BENCH_ORDER` accepts `sequential-first` or `grouped-first` so
repeated runs can alternate order. `WSLDB_GROUP_BENCH_KEEP=1` retains the two
databases; otherwise the harness removes only its exact main, lock, and WAL
paths after verification.

## Full-verification diagnostic

The verification-only harness isolates W7 full structural, checksum, value,
and logical-hash verification from sustained-update execution. It does not
issue application writes. Opening retains normal recovery semantics and may
truncate an incomplete or stale WAL before measurement. One warm-up and five
measured opens are the defaults; every run must return an identical
`VerificationReport`.

```sh
WSLDB_RUN_VERIFY_BENCH=1 \
WSLDB_VERIFY_DATABASE=/path/to/existing.wsldb \
  cargo bench --bench verification
```

`WSLDB_VERIFY_RUNS`, `WSLDB_VERIFY_WARMUP_RUNS`, and
`WSLDB_VERIFY_CACHE_PAGES` override the recorded configuration. The measured
interval begins after open and ends after `Database::verify`; startup and WAL
replay therefore remain separate W7 measurements. Run order, OS cache state,
filesystem, hardware, executable hash, and process resource usage must still be
captured by the surrounding comparison script before results are used as
release evidence.

## WAL file-growth diagnostic

The opt-in WAL sync harness isolates filesystem synchronization while a file
grows from synchronization while the same offsets of an already allocated
extent are overwritten. Each path writes identical 256-byte records and calls
`File::sync_all` after every record. Initialization is outside the measured
interval, run order alternates, and both files are read back in full.

```sh
WSLDB_RUN_WAL_SYNC_BENCH=1 \
WSLDB_WAL_SYNC_DIRECTORY=/path/on/test/filesystem \
  cargo bench --bench wal_sync
```

`WSLDB_WAL_SYNC_RECORDS` and `WSLDB_WAL_SYNC_RUNS` default to 64 and 3.
`WSLDB_WAL_SYNC_ORDER` accepts `growing-first` or `fixed-first`. The harness
uses independent temporary files and removes only its exact paths after each
verified measurement. It diagnoses one filesystem behavior and is not a
database throughput result.

## Online compaction diagnostic

The opt-in compaction harness starts active-file compaction of an existing
buffered database while the original handle issues durable overwrites. It
reports how many commits overlapped the rebuild, the expected generation
conflict at publication, compaction wall time, the publication-lock pause,
overlapping commit p99/maximum, and logical storage before and after. The final
reopened snapshot must match the compaction report exactly.

```sh
WSLDB_RUN_COMPACTION_BENCH=1 \
WSLDB_COMPACTION_DATABASE=/path/to/existing.wsldb \
  cargo bench --bench compaction
```

Use a disposable clone because successful active compaction replaces the main
file generation. This bounded diagnostic does not replace W5/W6 or measure an
automatic compaction policy.

## Sustained RocksDB comparison

`rocksdb_sustained_bench.cc` implements W5 and W6 through the public RocksDB
API with the same seed, byte-identical keys, values, operation selector,
uniform/Zipf samplers, and durable delete/recreate cycles as the WSLDB harness.
Both adapters encode the selected key before starting the per-operation timer;
sampler and key-generation CPU time is therefore excluded symmetrically while
database transaction acquisition and all database work remain measured.
The RocksDB cycle performs an exact `Get` before `Delete`, matching
`WriteTransaction::delete` rather than relying on RocksDB's successful
missing-key delete status.
Before the timed warm-cache profile, both adapters traverse the complete
logical dataset and validate its entry count. This symmetry matters because a
RocksDB iterator populates its block cache, while a WSLDB metadata count does
not touch its decoded-page cache.
Every RocksDB mutation enables WAL and requests a synchronous write. Both
adapters perform and separately report periodic full verification, report the
final RocksDB flush plus wait for scheduled background work or WSLDB
write-buffer flush as a separate drain metric, verify
checksums, exact entry count, and the same logical SHA-256 stream, and leave no
unmeasured maintenance work. Both emit the same one-second windows and exact
whole-run `_total` latency distributions. Each adapter starts the next active
window only after emitting the previous report and completing any periodic
verification or WSLDB checkpoint/compaction instrumentation, so benchmark-only
bookkeeping is excluded from the active throughput denominator.
The comparison runner configures the same positive write-buffer threshold for
both engines and records it separately for WSLDB and RocksDB. The frozen
default is 64 MiB, matching the buffered WSLDB profile used by the preregistered
sustained work and replacing reliance on RocksDB's version-dependent default.
WSLDB setup remains direct because loading is outside the sustained interval;
the measured WSLDB reopen and both RocksDB phases receive the recorded value.

Timed runs naturally finish different operation counts and therefore different
logical states. Setting `WSLDB_SUSTAINED_OPERATION_LIMIT` makes the runner pass
the same deterministic operation limit to both adapters and fail unless their
final operation count, entry count, and logical SHA-256 are identical. Leave it
unset for the preregistered duration-based performance comparison.
Before either timed process starts, the runner also requires exactly one
`load_complete` record from each adapter, the configured entry count, a
well-formed SHA-256, and identical logical hashes. Every duration-based process
must finish with exactly one `sustained_complete` record, a positive operation
count, the configured entry count, and a well-formed final SHA-256. Missing,
duplicated, malformed, or contradictory records invalidate the run instead of
leaving partial output that could be mistaken for benchmark evidence.

Build the adapter against the same pinned RocksDB checkout and static library
as `db_bench`:

```sh
c++ -std=c++20 -O3 -Wall -Wextra -Wpedantic -Werror \
  -I"$ROCKSDB/include" -I"$ROCKSDB" \
  benchmarks/rocksdb_sustained_bench.cc "$ROCKSDB/librocksdb.a" \
  $ROCKSDB_PLATFORM_LIBS -o /path/to/rocksdb_sustained_bench
```

Setting `WSLDB_ROCKSDB_WAL_STATS=1` attaches RocksDB's built-in statistics to
the sustained adapter and reports exact WAL sync count, bytes, and
`WAL_FILE_SYNC_MICROS` distribution on standard error. It is an opt-in
diagnostic because statistics collection adds overhead; frozen comparisons
leave it unset.

Run the preregistered comparison with one warm-up plus five measured 30-minute
processes per engine and distribution:

```sh
benchmarks/run-sustained-comparison.sh \
  /path/to/rocksdb_sustained_bench "$ROCKSDB" /new/evidence/directory
```

The `WSLDB_SUSTAINED_*` environment controls recorded by the runner can reduce
duration, distributions, repetitions, or set an alternate positive write
buffer for diagnostic work. The selected write buffer is applied to both
measured engines. Such altered runs do not satisfy the frozen production gate.
The sustained runner uses the same strict
release-mode checkout and version contract as the short runner; its adapter
supports `--version` so the linked RocksDB library is checked before execution.
It rejects an empty or duplicated distribution list, validates numeric inputs
before shell arithmetic, and removes inherited operation-limit, load-drain,
write-buffer, and RocksDB-statistics controls before installing the recorded
values for each child process. Setup always uses a direct WSLDB tree, while the
measured process receives the manifest's configured write-buffer value.
Manifests record the deterministic seed, RocksDB WAL-statistics mode, scratch
filesystem, mount table, CPU, memory, storage, kernel, and toolchain context.
For every WSLDB process, the runner fail-closed joins each `w5_all` or `w6_all`
window with its checkpoint-activity record and the available overwrite and
delete p99 values. The resulting `checkpoint-windows.csv` identifies workload,
outer run, one-second window, starts, completions, failures, bytes, in-progress
work, mutation counts and p99 values, total operations, and throughput. Missing,
duplicated, non-contiguous, malformed, or internally contradictory window
records invalidate the run;
windows containing no sample of one mutation kind retain an empty p99 rather
than inventing a zero-latency observation. This derived file is an audit aid;
the raw stdout and stderr remain authoritative evidence.

Both comparison runners record SHA-256 values for their own orchestration
script, shared benchmark contract, common evidence-hash writer, and common
verifier; the sustained runner also records the window-join script. Only after
every requested process, validation, size measurement, and cleanup succeeds do
they append `runner_complete=yes` and create `evidence-sha256.txt`. That final,
path-sorted file hashes every regular evidence file other than itself,
including the manifest, commands, raw logs, resource reports, size table, and
derived checkpoint windows. The runner then checks every digest, rejects
missing or unlisted files, duplicate records, unsafe paths, symbolic links,
and non-regular entries. Verification failure removes the seal. A failed or
interrupted run therefore has no final hash manifest and cannot be treated as
complete. Before sealing, release-mode runs also revalidate both clean source
commits and every hashed runner, adapter, benchmark, reset hook, and evidence
tool used by that workload. WSLDB release binaries are compiled from a private
detached clone of the recorded commit inside runner scratch space, rather than
from mutable working-tree files. Any mid-run input change invalidates the run.
After transfer, rerun
`scripts/verify-evidence-hashes.sh /path/to/evidence` before analysis or
archival.
