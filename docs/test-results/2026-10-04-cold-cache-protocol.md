# Isolated cold-cache comparison protocol — 2026-10-04

## Subject

- WSLDB commit: `e43a40b92dc33f24266b5ff6082e72ae27505678`
- Covered workloads: W3 point reads, W4 ranges, and clean W7
  open-to-first-read
- Processes per engine and profile: one discarded r0 plus measured r1–r5
- Cache-reset interface: executable invoked as `COMMAND DATABASE LABEL`
- Reference helpers: macOS `sync` plus `/usr/sbin/purge`; Linux `sync` plus
  `/proc/sys/vm/drop_caches`

## Mechanism

Cold mode creates a new engine process for every discarded or measured run.
Before each process, it invokes the configured reset executable and captures
its exact command, stdout, stderr, and SHA-256. WSLDB disables its in-process
warm-up, creates a fresh decoded-page cache on open, and executes exactly one
globally numbered run. RocksDB likewise executes one benchmark repetition per
process. Engine order alternates between adjacent pairs.

The warm path remains the default and preserves its existing in-process
warm-up and five measured repetitions. The Rust measurement-plan defaults are
covered separately from the cold single-process configuration.

The reference helpers reject the wrong operating system, a missing database,
missing host utilities, or unavailable non-interactive privilege. `--check-host`
validates only platform support for local release hygiene. `--check` also
requires privilege readiness without performing an eviction. Runtime never
prompts: an expired or absent authorization fails the run before the next
engine process.

## Verification

The following focused checks passed on the subject source:

```text
scripts/release-audit.sh
bash -n benchmarks/cache-reset-macos.sh benchmarks/cache-reset-linux.sh
benchmarks/cache-reset-macos.sh --check-host
cargo fmt --all -- --check
cargo test --test benchmark_measurement --all-features
cargo check --bench wsldb --all-features
cargo clippy --bench wsldb --test benchmark_measurement \
  --all-features -- -D warnings
git diff --check
```

The shell integration test executes the complete comparison front end in cold
mode with deterministic stub engines and a reset recorder. It observes exactly
36 reset calls for one value size and one W3/W4 variant: six processes times
two engines times three workloads. It also verifies r0 order
WSLDB-then-RocksDB, r1 order RocksDB-then-WSLDB, the one-run/no-warm-up WSLDB
environment, the reset executable hash, and manifest cache-state fields.

## Scope

No operating-system cache was evicted and no timed database result was
collected for this change. A stub reset proves orchestration, not APFS, ext4,
or XFS cache behavior. Release evidence must independently demonstrate that
the hashed platform command evicted the tested main file, WSLDB sidecar where
present, and RocksDB files before every recorded process. The cold-cache
production gate therefore remains open.
