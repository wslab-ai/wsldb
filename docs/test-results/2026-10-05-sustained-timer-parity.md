# Sustained operation timer parity

Source commit under test: `c18a600` (`fix: equalize sustained timer
boundaries`).

## Defect

Both W5/W6 adapters choose the same index and encode the same 16-byte key, but
their latency boundaries differed. RocksDB built `Key(index)` and its `Slice`
before `operation_started`; WSLDB called `key(index)` from inside every timed
`get`, `put`, and `delete` path. WSLDB therefore paid deterministic benchmark
key-encoding CPU time in point-read and mutation samples that RocksDB excluded.

This did not alter logical hashes, entry counts, durability, or operation
selection, so semantic conformance alone could not detect the measurement
bias.

## Corrected contract

WSLDB now encodes one `operation_key` immediately before starting the
per-operation timer and reuses it for all database calls in that operation.
The timer continues to include transaction acquisition, lookup, value
generation for mutations, WAL-backed commits, and delete/recreate work. This
matches the existing RocksDB boundary without weakening either engine's
durability path.

The dependency-free benchmark contract pins both source orderings: WSLDB key
encoding must immediately precede its timer, RocksDB key plus slice encoding
must precede its timer, and every WSLDB database call must use the shared
pre-encoded key.

All sustained performance results collected before this correction remain
historical diagnostics only. No improvement ratio is claimed without a new
pinned, alternating external run.

## Verification

```text
cargo fmt --all -- --check
  passed

bash -n benchmarks/benchmark-contract.test.sh \
  benchmarks/run-sustained-comparison.sh
bash benchmarks/benchmark-contract.test.sh
  benchmark contract tests passed

CARGO_TARGET_DIR=/tmp/wsldb-timer-check.<temporary> \
  cargo check --bench wsldb
  Finished `dev` profile

CARGO_TARGET_DIR=/tmp/wsldb-timer-check.<temporary> \
  cargo clippy --bench wsldb -- -D warnings
  Finished `dev` profile

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

git diff --check
  passed
```

The temporary Cargo target was removed after the checks, and the repository
has no `target` directory. No database benchmark, cache reset, reboot,
shutdown, sleep, raw-device operation, or physical power-loss test was run on
the development laptop. The pinned W1-W7 matrix and the other external release
gates remain open.
