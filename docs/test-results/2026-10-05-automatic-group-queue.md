# Automatic group-commit queue scaling

Date: 2026-10-05

## Risk and change

The automatic group-commit coordinator kept pending callers in a `Vec`.
Removing a bounded group from the front shifted every remaining caller, and
skipping disconnected waiters during leadership handoff repeatedly removed
index zero. A large concurrent queue could therefore perform quadratic work
while holding the coordinator mutex, delaying unrelated callers.

The queue now uses `VecDeque`. Enqueue, front removal, and disconnected-waiter
skipping are constant-time; draining a group performs work proportional only
to that group. The existing 65,536-transaction physical-frame limit, FIFO
ordering, per-caller transaction IDs, fallible result staging, leader-unwind
behavior, and fail-closed error propagation are unchanged.

No throughput improvement is claimed from this structural change. Comparative
performance still requires the frozen concurrent RocksDB matrix.

## Verification

A focused test queues and drops 4,096 disconnected waiters behind an active
leader, adds one connected waiter, and proves that handoff skips the abandoned
prefix and promotes the remaining caller in FIFO order. Existing unit tests
also cover bounded group splitting, failures, and leader unwind.

```text
$ CARGO_BUILD_JOBS=2 cargo test --locked --lib application::group_commit::tests
running 7 tests
test result: ok. 7 passed; 0 failed

$ CARGO_BUILD_JOBS=2 cargo test --locked --test group_commit --all-features
running 12 tests
test result: ok. 12 passed; 0 failed

$ CARGO_BUILD_JOBS=2 cargo check --locked --all-targets --all-features
Finished `dev` profile

$ CARGO_BUILD_JOBS=2 cargo clippy --locked --all-targets --all-features -- -D warnings
Finished `dev` profile

$ CARGO_BUILD_JOBS=2 cargo test --locked --all-targets --all-features
275 Rust tests passed; 5 opt-in benchmark entrypoints exited without running workloads
```

No soak, benchmark, cache reset, reboot, shutdown, sleep, or power operation
was run for this verification.
