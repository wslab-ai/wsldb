# Checkpoint-window evidence join

Date: 2026-10-05

Source commit: `4639f3e` (`bench: join checkpoint activity windows`).

## Problem

The sustained WSLDB adapter emitted per-second latency CSV on standard output
and checkpoint lifecycle records on standard error. The comparison runner kept
both raw files, but did not establish a one-to-one join. A missing, duplicate,
or malformed lifecycle line could therefore make later checkpoint/tail
analysis incomplete without invalidating the benchmark process.

## Contract

`benchmarks/checkpoint-window-report.sh` joins each W5/W6 total window with its
checkpoint record and any overwrite or delete p99 metrics. It emits, in window
order:

- workload, outer process run, and one-second window number;
- activity flag, starts, successful publications, failures, bytes, and work in
  progress;
- overwrite and delete operation counts plus p99 when that mutation kind was
  present; and
- total operations and throughput.

The parser requires positive contiguous window numbers, one total metric and
one complete checkpoint record per window, unique optional mutation metrics,
numeric fields, and a binary activity flag. A window with no sample of a
mutation kind records count zero and an empty p99; it does not invent a zero
latency. An inactive flag cannot accompany lifecycle deltas, bytes, or current
work. Any other missing, duplicated, malformed, or contradictory record fails
the complete sustained runner before scratch data is removed.

The runner writes the joined rows to `checkpoint-windows.csv` while retaining
raw stdout and stderr as the authoritative evidence. Warm-up and every measured
process are distinguished by their outer run number. This report does not
choose an optimization or claim correlation; it makes the preregistered
checkpoint-interference question reproducibly answerable by the external
alternating matrix.

## Verification

No database benchmark, cache reset, reboot, shutdown, sleep, or power operation
ran. The deterministic fixture covers a valid joined window, a window without
delete samples, both outer runs, and rejection of incomplete and contradictory
checkpoint records.

```text
bash -n benchmarks/checkpoint-window-report.sh \
  benchmarks/run-sustained-comparison.sh \
  benchmarks/benchmark-contract.test.sh scripts/release-audit.sh

benchmarks/benchmark-contract.test.sh
  benchmark contract tests passed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

git diff --check
  passed
```

The pinned alternating W5/W6 run is still required. Until that external
evidence exists, checkpoint scheduling remains an untested hypothesis and the
documented RocksDB performance gap remains open.
