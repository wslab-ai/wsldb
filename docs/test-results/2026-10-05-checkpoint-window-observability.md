# Checkpoint-window observability

Source commits under test:

- `4ae97e3` (`feat: expose checkpoint activity`);
- `f8fd3ac` (`feat: attribute checkpoint bytes`);
- `d64d02d` (`fix: track checkpoint window activity`);
- `1e95fed` (`fix: drain checkpoints before benchmark compaction`).

## Risk and contract

The sustained W5/W6 harness recorded per-second mutation latency but exposed
only cumulative checkpoint failures through `Database::operational_stats`.
That call performs a full uncached tree and value verification, so polling it
every second would perturb the workload whose tail latency it was intended to
explain. Successful checkpoint publication had no independent counter.
Consequently, existing evidence could not identify which mutation windows
overlapped checkpoint work.

`Database::checkpoint_stats` now returns a process-local `CheckpointStats`
without reading database pages. It contains:

- monotonic checkpoint-start and current in-progress counters;
- a bounded `LatencyStats` histogram for successful primary-tree checkpoints;
- the existing automatic-checkpoint failure count;
- cumulative primary-tree data-page and metadata bytes.

A checkpoint becomes successful only after its metadata sync completes. Its
end-to-end duration starts when tree preparation begins and ends after durable
metadata publication, including background preparation and any wait before
publication. A WAL snapshot consolidation does not increment the tree-
checkpoint histogram. A checkpoint that durably publishes metadata but later
reports an ambiguous WAL-cleanup outcome remains a successful checkpoint,
because recovery may select that physical root.

`OperationalStats` includes the same start, in-progress, and successful values
for complete audits. A lifetime guard keeps `in_progress` nonzero throughout
background preparation, the wait for foreground publication, and metadata
publication; every success, ordinary error, worker panic, and thread-spawn
failure drops the guard.

The sustained benchmark samples the cheap view after each one-second latency
window. A window is marked active if a checkpoint starts or completes inside
it, or if work is active at either boundary. Delta and cumulative start,
successful/failure, and byte counters are emitted with the current in-progress
count. Verification/drain activity is reported separately, and cumulative
counts survive the fresh handle installed after active compaction rather than
silently resetting with its process-local metrics. Before that replacement,
the harness explicitly measures and drains pending checkpoint work and fails
closed if the active count is not zero, so a detached worker cannot overlap
compaction as hidden I/O. The final drain enforces the same invariant. The
final report emits checkpoint starts, active count, successful count, total,
maximum and p99 duration, and bytes. No database or WAL byte changed.

The counter uses one existing bounded logarithmic histogram plus two atomics.
All eight runtime histograms together remain below the existing 64 KiB unit-
test limit. A normal commit performs no new work unless it schedules or
publishes a checkpoint; monitoring reads only bounded relaxed atomics.

## Verification

Build concurrency was limited to two jobs. Benchmark and opt-in soak/fuzz
bodies were not enabled.

```text
CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  246 Rust tests and 5 opt-in benchmark entrypoint checks passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test write_buffer --all-features
  16 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test maintenance_failures --all-features
  11 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test freeze_swap_checkpoint --all-features
  3 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test public_api --all-features
  3 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' CARGO_BUILD_JOBS=2 \
  cargo doc --no-deps --all-features
scripts/release-audit.sh
cargo fmt --all -- --check
git diff --check
  passed
```

The complete Rust suite above ran after the checkpoint lifecycle changes at
`d64d02d`. The later benchmark-orchestration-only change at `1e95fed` changed
no library or test source and additionally passed:

```text
CARGO_BUILD_JOBS=2 cargo check --bench wsldb --all-features
CARGO_BUILD_JOBS=2 cargo clippy --bench wsldb --all-features -- -D warnings
scripts/release-audit.sh
cargo fmt --all -- --check
git diff --check
  passed
```

Focused assertions prove that background preparation is visible as one active
checkpoint before publication, success returns the active count to zero and
increments written bytes, explicit WAL consolidation leaves every tree-
checkpoint counter at zero, and a worker panic returns the active count to zero
while exposing one failed attempt without losing the already durable
transaction. The external-consumer compile contract covers the new public
type, method, fields, and trait guarantees; its pinned SHA-256 was intentionally
updated.

No timed benchmark, cache reset, reboot, shutdown, sleep, power operation,
raw-device access, or physical power-loss test was performed. This change
enables the next causal W5/W6 comparison; it is not evidence of a performance
improvement or RocksDB parity. Linux, cold-cache, physical power-loss, and the
pinned RocksDB release matrix remain open.
