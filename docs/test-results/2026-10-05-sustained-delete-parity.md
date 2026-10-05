# Sustained delete/recreate parity

Source commit under test: `0d353c3` (`fix: equalize sustained delete
semantics`).

## Defect

The W5/W6 adapters both time a delete followed by a recreate as one operation,
but they did not execute equivalent work. WSLDB calls
`WriteTransaction::delete`, which performs an exact point read and returns
whether the selected key exists before staging the tombstone. The harness fails
if that result is false.

The RocksDB adapter previously called `DB::Delete` directly. Its successful
status confirmed that the delete was accepted, not that the key had existed,
so it neither enforced the fixed-size workload invariant at that boundary nor
paid the corresponding point-lookup cost. The final entry count and logical
hash could detect many persistent divergences, but they could not make the
timed operations symmetric.

## Corrected contract

Inside the same delete/recreate latency interval, the RocksDB adapter now:

1. performs an exact `DB::Get` for the selected key and fails on `NotFound` or
   any other non-OK status;
2. executes the synchronous WAL-backed delete;
3. deterministically rebuilds the value and executes the synchronous
   WAL-backed recreate.

The internal existence read is not added to the explicit 80% point-read byte
counter, matching WSLDB where it is part of delete semantics rather than a
separate workload operation. Operation selection, seed, keys, values,
durability, window boundaries, and final logical verification are unchanged.

Every RocksDB sustained result recorded before this correction remains useful
as historical diagnostic evidence, but cannot be included in the required
pinned alternating release matrix. This correction is expected to move the
comparison in WSLDB's favor, but no ratio is claimed until new external runs
are collected.

## Verification

The dependency-free benchmark contract checks that the exact existence check
is present immediately before the durable delete. It also exercises the
operation-limited runner with stubs whose final operation count, entry count,
and logical hash must agree.

```text
bash -n benchmarks/benchmark-contract.test.sh \
  benchmarks/run-sustained-comparison.sh
benchmarks/benchmark-contract.test.sh
  benchmark contract tests passed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

git diff --check
  passed
```

The adapter was not linked locally against the pinned RocksDB 11.8.1 source;
that compile and the repeated W5/W6 executions belong on the isolated external
benchmark host. No database workload, cache reset, reboot, shutdown, sleep,
power operation, raw-device access, or physical power-loss test was performed.
