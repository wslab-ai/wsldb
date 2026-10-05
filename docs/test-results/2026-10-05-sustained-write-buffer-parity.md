# Sustained write-buffer parity

Source commit under test: `b8c37d2` (`fix: align sustained write buffers`).

## Defect

The sustained runner recorded a WSLDB write-buffer setting but defaulted it to
zero, selecting direct copy-on-write commits. RocksDB retained its implicit
version-specific memtable size. The published short and long W5/W6 diagnostics
had explicitly selected WSLDB's 64 MiB durable buffer, so invoking the
documented runner without extra environment configuration did not reproduce
the intended profile.

This was both a performance and methodology defect: WSLDB and RocksDB could be
timed with different foreground/background persistence architectures, and the
RocksDB buffer was absent from the manifest.

## Corrected contract

- The runner defaults to a positive 64 MiB write buffer, the already
  preregistered buffered WSLDB profile.
- It removes any inherited `WSLDB_BENCH_WRITE_BUFFER_BYTES` value before both
  measured processes and installs the one manifest value explicitly.
- The RocksDB adapter parses the same value and assigns
  `Options::write_buffer_size`; it uses 64 MiB when invoked standalone without
  the environment variable.
- The manifest records `write_buffer_bytes` and
  `rocksdb_write_buffer_bytes` independently plus disabled RocksDB compression.
- A zero threshold is rejected by the comparison runner. WSLDB setup remains
  direct and outside timing, while the measured reopen uses the shared value.
- Release mode rejects any value other than 67,108,864 bytes; alternate
  positive values are diagnostic-only.

Changing the positive value remains possible in diagnostic mode and is
recorded. No performance claim is inferred from correcting the configuration.

## Verification

The executable stubs begin with an inherited value of 999, then require direct
WSLDB setup and exactly 67,108,864 bytes for measured WSLDB and for both RocksDB
phases. The contract also verifies the manifest, exact sanitized command, and
failure on a zero requested buffer.

```text
bash -n benchmarks/run-sustained-comparison.sh \
  benchmarks/benchmark-contract.test.sh
bash benchmarks/benchmark-contract.test.sh
  benchmark contract tests passed

c++ -std=c++20 -Wall -Wextra -Wpedantic -Werror -fsyntax-only \
  -I/opt/homebrew/opt/rocksdb/include \
  benchmarks/rocksdb_sustained_bench.cc
  passed against the installed headers

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

git diff --check
  passed
```

The local syntax check does not replace compiling and linking against pinned
RocksDB 11.8.1 on the external benchmark host. No database benchmark, cache
reset, reboot, shutdown, sleep, raw-device operation, or physical power-loss
test was run on the development laptop. The corrected pinned W1-W7 matrix and
all other external release gates remain open.
