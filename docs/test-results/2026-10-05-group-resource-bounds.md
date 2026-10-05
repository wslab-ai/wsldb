# Group-commit resource-bound verification — 2026-10-05

Source commit under test:
`6a811bf6f36f8be361058925fae229f21ec0d50d`
(`fix: bound group commit resource use`).

## Risk and contract

The compact WAL frame stores its transaction count as a `u32`. Payload-length
checks prove that the bytes can encode the advertised transactions, but an
otherwise valid one-gibibyte frame could still contain hundreds of millions of
empty transactions. Recovery previously reserved a `Vec<WalRecord>` directly
from that count, allowing memory use to grow far beyond the persisted bytes.

One physical group is now limited to 65,536 logical transactions. Explicit
`Database::commit_group` calls reject a larger vector before taking the writer
lock. Automatic `commit_batch` keeps accepting callers and drains a larger
pending queue as consecutive bounded groups, preserving FIFO order and giving
each caller its own transaction ID. Encoding independently enforces the same
limit, and recovery rejects a checksummed frame that advertises more. The
record representation and WAL force ordering are unchanged.

`MAX_GROUP_TRANSACTIONS` exposes the limit at the crate root. The external
consumer contract exercises the export, and its deliberately pinned SHA-256
was updated to
`7a88aa5cba92c80e171da2ae6546c7b1f62894b08b001d8b400ba404f230f0a0`.

Focused tests prove the limit check without allocating 65,537 batches, prove
bounded queue draining and leadership handoff with a small test limit, and
mutate a valid group header above the limit before recomputing every checksum.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
CARGO_BUILD_JOBS=2 cargo test --lib group_transaction_limit
CARGO_BUILD_JOBS=2 cargo test --lib bounded_groups
CARGO_BUILD_JOBS=2 cargo test --lib transaction_count_above_the_resource_limit
  3 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  223 tests passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" CARGO_BUILD_JOBS=2 cargo doc --no-deps --all-features
  passed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

This closes the unbounded transaction-vector amplification path for physical
group frames. It does not close the external Linux, filesystem power-loss, or
pinned RocksDB release gates.
