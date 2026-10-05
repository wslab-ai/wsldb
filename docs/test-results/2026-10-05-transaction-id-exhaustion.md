# Transaction-ID exhaustion preflight

Date: 2026-10-05

## Risk

Transaction IDs are unsigned 64-bit monotonic counters. Single commits compute
the next ID before calling the storage adapter. Group commits previously
reserved their complete ID vector before discovering arithmetic overflow while
constructing it. No focused test proved that overflow on the first or a later
group member rejects the whole operation before storage receives any mutation.

## Coverage

Group ID construction now proves the final successor fits in `u64` before
allocating its result vector. The application test adapter counts single and
group commit calls, and boundary tests install synthetic valid heads at the end
of the ID space:

- a single commit from `u64::MAX` returns `TransactionIdExhausted`;
- a two-batch group from `u64::MAX - 1` fails when the second successor would
  overflow;
- two consecutive automatic `commit_batch` calls at `u64::MAX` both return
  promptly with `TransactionIdExhausted`, proving leadership is handed back
  after the first ordinary preflight error.

All cases prove the adapter call count stays zero and the database head remains
unchanged. The group cannot publish the first batch as a partial prefix, and
deterministic ID exhaustion takes precedence over allocation failure. No
persisted format or public API changed.

## Verification

All Cargo commands used at most two jobs and a private temporary target
directory that was deleted after completion.

```text
cargo test --all-features --lib transaction_id_exhaustion
  3 passed

cargo test --all-targets --all-features
  112 library tests passed
  159 integration tests passed
  2 power-loss probe tests passed
  5 benchmark entrypoints remained opt-in

cargo check --all-targets --all-features
  passed

cargo clippy --all-targets --all-features -- -D warnings
  passed

RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
  passed

cargo fmt --all -- --check
  passed

git diff --check
  passed

scripts/release-audit.sh
  passed
```

The complete executed suite contained 273 Rust tests. No benchmark workload,
opt-in soak body, corruption-fuzz body, cache reset, reboot, shutdown, sleep,
raw-device, or power operation ran.
