# WAL pre-sync process-crash boundary

Date: 2026-10-04.

## Change

The WAL append path now exposes a test-only crash boundary after the complete
positioned write and before the mandatory sync. This closes the previously
unexercised process-crash interval between a successful write and durable
confirmation. The hook compiles to a no-op when the `crash-injection` feature
is disabled and changes neither the persisted format nor the public API.

Two subprocess tests cover the boundary:

- an ordinary buffered commit must recover either the complete previous value
  or the complete new value;
- a two-transaction group must recover either neither transaction or both
  transactions, never a complete prefix or other partial group.

The existing post-sync group test still requires the complete new group, so
the pre-sync allowance does not weaken the durable-success contract.

## Verification

The worktree based on commit `0f310c9` passed:

```text
cargo test --all-targets --all-features
cargo test --release --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets --all-features
cargo fmt --all -- --check
RUSTDOCFLAGS='-D warnings' cargo doc --all-features --no-deps
cargo package --list --allow-dirty
cargo package --allow-dirty
git diff --check
```

Both debug and release suites executed 177 tests with no failures. The focused
pre-sync ordinary, pre-sync group, and post-sync group tests also passed in
isolation before the complete suites. Rustdoc passed with warnings denied. The
package selected 196 files and independently compiled its unpacked source; the
240,360-byte archive SHA-256 is
`dfebe191939c6aa6bd4e331ce7965da65991d25347eaf7a7eb9693398223a88d`.
The exact package-list output SHA-256 is
`13fc58d22a23b271d3fa2a10bd0d03bf94522c69ebfe1705571bf4393f6bbf9c`.

This result covers abrupt process termination with the host kernel and
filesystem still running. It does not emulate a lost device cache, reordered
stable writes, torn sectors, a kernel crash, or physical power removal; the
filesystem-specific power-loss gate remains open.
