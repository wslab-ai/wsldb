# Fallible group-commit staging verification — 2026-10-05

Source commit under test:
`fdc05bfe820f3ff6b6f5caa8886d4c971ae188b3`
(`fix: make group staging fallible`).

## Risk and contract

Group WAL frames were already limited to 65,536 logical transactions, but the
application layer still used ordinary `Vec::push` or iterator collection for
three transaction-count-sized staging buffers. Allocation failure in those
buffers could therefore terminate the process before the bounded group reached
the storage engine.

The coordinator now reserves pending-queue capacity before enqueueing, and it
reserves the extracted-group vector before draining any request. Explicit and
automatic groups reserve their change-set vector before moving a batch into it.
The automatic path feeds its exact-size batch iterator directly into that
staging step, removing the previous intermediate batch vector.

If group extraction cannot reserve memory, every request currently in the
queue receives `Error::FileTooLarge`. The active leadership guard remains the
sole owner of handoff state: a request arriving during rejection stays queued,
is promoted after the guard exits, and can complete normally. This ordinary
preflight failure does not poison subsequent groups. No storage-engine call or
WAL write occurs before these staging allocations succeed.

The focused coordinator regression exercises rejection of two waiters, an
arrival concurrent with rejection, leadership transfer, successful completion
of the transferred request, and a later independent leader. Existing explicit
and automatic group tests continue to cover transaction ordering, shared
syncs, recovery, crash boundaries, injected I/O errors, stale heads, and
disabled buffering.

This change narrows a bounded contiguous-allocation failure path. It is not a
claim that every allocator failure inside Rust collections, channels, or the
operating system is recoverable.

## Verification

Cargo work was limited to two build jobs. Benchmark, soak, physical power-loss,
shutdown, and reboot modes were not invoked.

```text
CARGO_BUILD_JOBS=2 cargo test --lib application::group_commit::tests --all-features
  6 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test group_commit --all-features
  12 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  228 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" CARGO_BUILD_JOBS=2 cargo doc --no-deps --all-features
cargo fmt --all -- --check
git diff --check
  passed

CARGO_BUILD_JOBS=2 scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed
```

This closes the remaining transaction-count-sized infallible vectors in the
application group-staging path. It does not close the external Linux,
filesystem power-loss, cold-cache, or pinned RocksDB release gates.
