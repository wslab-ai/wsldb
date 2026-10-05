# Sustained active-window accounting

Source commit under test: `5e08f4f` (`fix: exclude sustained reporting
overhead`).

## Defect

At each one-second W5/W6 boundary, both adapters closed and reported the active
window, then set the next window start to the timestamp captured before that
report. Sorting latency samples, formatting output, and periodic verification
were therefore charged to the next throughput denominator even though no
workload operation ran during that interval.

The distortion was asymmetric in practice: WSLDB additionally reads and emits
checkpoint activity counters at every boundary. Maintenance windows already
had a later reset, but ordinary windows did not. Logical contents, latency
samples, and operation selection could all remain correct while total active
throughput was understated.

## Corrected contract

Both adapters now reset `window_started` only after all report emission and
optional maintenance for the completed window. Consequently:

- operation latency still includes all intended database work;
- `_total` active elapsed time is the sum of operation-bearing windows;
- benchmark reporting, hash verification, checkpoint observation, compaction,
  and maintenance reporting remain outside that denominator;
- the wall-clock deadline, operation stream, durability, and final verification
  are unchanged.

The dependency-free contract rejects either adapter if it regains an
intermediate `window_started` assignment that would open the next active window
before instrumentation completes.

No performance delta is inferred from this accounting correction. All prior
sustained ratios remain historical until the pinned alternating external matrix
is rerun.

## Verification

```text
cargo fmt --all -- --check
  passed

bash -n benchmarks/benchmark-contract.test.sh \
  benchmarks/run-sustained-comparison.sh
bash benchmarks/benchmark-contract.test.sh
  benchmark contract tests passed

CARGO_TARGET_DIR=/tmp/wsldb-window-check.<temporary> \
  cargo check --bench wsldb
  Finished `dev` profile

CARGO_TARGET_DIR=/tmp/wsldb-window-check.<temporary> \
  cargo clippy --bench wsldb -- -D warnings
  Finished `dev` profile

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

git diff --check
  passed
```

The temporary Cargo target was removed after verification, and the repository
has no `target` directory. The RocksDB adapter was not linked locally against
the pinned checkout. No database benchmark, cache reset, reboot, shutdown,
sleep, raw-device operation, or physical power-loss test was run on the
development laptop. External performance and release gates remain open.
