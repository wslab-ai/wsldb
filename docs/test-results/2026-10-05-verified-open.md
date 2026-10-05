# Full verification on open

Date: 2026-10-05

## Risk and policy

Fast opening validates both metadata slots, the selected root page, and every
WAL record. It cannot prove that every deeper tree or overflow page is intact
without reading the complete live database. The previous README wording could
be read as promising a full scan while the implementation intentionally kept
startup proportional only to metadata, root, and WAL recovery.

WSLDB now exposes the tradeoff explicitly through
`DatabaseOptions::verify_on_open(bool)`. It defaults to `false`, preserving the
frozen fast-open workload and lazy checksummed-page validation. When enabled,
open performs the existing complete logical verification after main-file and
WAL recovery and before returning a database handle. A deep corruption error
therefore prevents an unchecked handle from escaping. Metadata or selected-root
damage continues to use the existing older-slot fallback before this optional
scan.

If the WAL was empty before recovery and the selected tree fails with a typed
corruption or depth error, strict open checks older root-valid metadata
candidates in descending order. It returns the newest candidate that passes a
complete scan. The corrupt slot is not rewritten: physical-generation
operations can continue to fail, so the returned older snapshot is a recovery
view. Writes and active maintenance fail closed instead of publishing over the
damaged generation. `backup` and `compact_to` revalidate the physical source
generation under the cross-process lock and can export the recovery head into
a new fully verified database. If a WAL was nonempty, fallback is prohibited
even when an older tree verifies, because the WAL may contain newer durable
transactions.

The option does not change persistent bytes. Because it adds a promised public
builder method, the external-consumer contract and its explicit release-audit
SHA-256 were updated to:

```text
cfb85f20e7111a03af5479c646a39d5cf474f0d6ad4cb9935c0313ba9d0dbcef
```

## Coverage

One direct test corrupts the first committed snapshot's overflow page while
leaving metadata and the root valid. Ordinary open still returns a handle whose
read and explicit verify produce `Error::Corrupt`; verified open returns the
fully verified initial empty transaction zero from the other slot. A
two-generation test corrupts only the newest generation and proves strict
fallback returns the older transaction and values while
physical operational statistics continue to expose the damaged current
generation. The same test exports that older view through both raw snapshot
backup and logical compaction, then strictly reopens and verifies both outputs.
A buffered fallback test proves a rejected mutation leaves the WAL empty and
that `compact_to` still exports the older snapshot rather than the corrupt
physical head. A matching nonempty-WAL test proves the corruption returns
`Error::Corrupt` instead of falling back. The ordinary buffered recovery test
also reopens a valid main+WAL logical snapshot with full verification enabled,
proving that verification occurs after WAL recovery. The external-consumer
test compiles the option as public API.

## Verification

All builds used at most two Cargo jobs in private temporary target directories
that were deleted after completion.

```text
cargo fmt --all -- --check
  passed

cargo test --all-features --test database \
  overflow_value_survives_reopen_and_is_checksummed
  1 passed

cargo test --all-features --test write_buffer \
  buffered_commits_preserve_snapshots_and_recover_without_options
  1 passed

cargo test --all-features --test public_api
  3 passed

cargo test --all-features --test verified_open
  3 passed

cargo check --all-targets --all-features
  passed

cargo clippy --all-targets --all-features -- -D warnings
  passed

RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
  passed

cargo test --all-targets --all-features
  106 library tests passed
  153 integration tests passed
  2 power-loss probe tests passed
  5 benchmark entrypoints remained opt-in

git diff --check
  passed

scripts/release-audit.sh
  passed
```

The complete executed total was 261 Rust tests. No performance workload,
cache reset, reboot, shutdown, sleep, or power operation ran. The default W7
startup contract was not changed and no new performance claim is made.
