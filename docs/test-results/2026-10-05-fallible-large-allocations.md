# Fallible large-allocation verification — 2026-10-05

Source commit under test:
`8bfb803c0b2b14526d352e90fce456442ffb9b2e`
(`fix: make large recovery allocations fallible`).

## Risk and change

WAL replay accepted physical records up to the documented 1 GiB limit but
created the body with an infallible zero-filled allocation. Allocation-capacity
failure could therefore terminate the process instead of returning a WSLDB
error. Similar potentially large reservations existed in overflow-value reads,
group preflight, merged buffered scans, transaction-ID result construction, and
the bounded streamed-snapshot writer.

These paths now reserve their existing capacities with `try_reserve` or
`try_reserve_exact` before initialization or use. Failure returns
`Error::FileTooLarge`; no persistent format, durability ordering, configured
limit, or public API changed. A focused unit test passes `usize::MAX` to the WAL
record-buffer allocator and requires the typed error without attempting the
allocation.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
CARGO_BUILD_JOBS=2 cargo test --lib
  74 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test group_commit --all-features
  12 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test write_buffer --all-features
  18 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  211 tests passed; 0 failed

scripts/release-audit.sh
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

This closes the identified capacity-overflow abort path. It does not claim
that an otherwise valid maximum-size record can be recovered without enough
memory to hold its logical contents, and it does not close the external Linux,
filesystem power-loss, or pinned RocksDB release gates.
