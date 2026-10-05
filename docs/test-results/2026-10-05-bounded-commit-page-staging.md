# Bounded commit-page staging verification — 2026-10-05

Source commit under test:
`b2cbee5b7e11d7554b49ac5b3b741c6d32586ec1`
(`refactor: bound commit page staging`).

## Risk and contract

Direct copy-on-write commits assign new virtual page IDs monotonically, retain
the corresponding nodes until reachability is known, and then write only the
reachable pages to dense physical IDs. The old implementation stored those
pages and cache markers in ordered trees, built traversal collections through
infallible growth, allocated a temporary reference vector per visited node,
and grew the virtual-to-physical remap after page writes had begun.

Virtual IDs within one `CommitContext` are already a dense interval. Pending
nodes and hot-cache markers now use parallel indexed vectors with fallible
capacity reservation before a virtual page is accepted. Reachability uses
pre-reserved indexed visited and postorder vectors; its explicit stack reserves
before every root, expanded-page, or child-reference insertion. Packed-leaf
overflow heads are visited through a borrowed iterator rather than an allocated
temporary vector.

Before the first physical page write, commit now also reserves and initializes
the complete dense remap vector. Every page reference resolves by checked
virtual-page offset in constant time. Unreachable virtual pages keep an empty
slot and consume no physical page. Page counters and the next physical ID are
validated before issuing each write. The streaming bulk writer similarly
reserves its bounded branch-cache staging slot before accepting a branch page.

A focused test deliberately creates one unreachable virtual leaf followed by
two reachable leaves and a branch. It proves that only three physical pages are
written, the physical range is dense, the root moves from virtual page 5 to
physical page 4, both branch references are remapped to their actual physical
IDs, and their logical leaf contents remain intact after decoding.

Page encoding, checksums, write/sync/metadata publication ordering, file format,
and public API are unchanged. Standard-library allocation inside bounded node
clones and cache structures is not claimed to be universally recoverable under
system-wide out-of-memory conditions. Any failure after an uncommitted page
write still leaves the older metadata snapshot authoritative.

## Verification

Cargo work was limited to two build jobs. Benchmark, soak, physical power-loss,
shutdown, and reboot modes were not invoked.

```text
CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::file_context::tests --all-features
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test database --all-features
  21 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --test append --all-features
  3 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  229 passed; 0 failed

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

This closes the identified transaction-sized infallible collections in direct
commit reachability and remapping. External Linux, filesystem power-loss,
cold-cache, and pinned RocksDB release gates remain open.
