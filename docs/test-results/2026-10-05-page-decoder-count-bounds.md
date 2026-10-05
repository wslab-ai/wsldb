# Page-decoder count-bound verification — 2026-10-05

Source commit under test:
`e6e272e848475ad958a73d69c58c1f44128d7773`
(`fix: bound page decoder allocations`).

## Risk and change

Leaf and branch counts are stored as `u16` fields inside a fixed 4 KiB page.
A corrupt page could advertise 65,535 entries while carrying only a few bytes
of payload. The leaf builder used fallible reservation, but still attempted a
capacity derived from that impossible count; the branch decoder used
infallible `Vec::with_capacity` before discovering truncation.

The decoder now proves each count against the remaining payload before any
count-sized collection allocation. The lower bounds are seven encoded bytes
per leaf entry and eleven bytes per valid branch separator/child pair. Branch
vectors then reserve fallibly, and separator lengths are rejected before
copying. These checks only reject encodings that could never fit in the page;
valid persisted bytes and the format are unchanged.

The focused mutation test begins with valid leaf and branch pages, replaces
their count fields with `u16::MAX`, recomputes both payload and page-header
checksums, and requires the count-bound corruption errors. Recomputing the
checksums proves rejection occurs in the structural decoder rather than at an
earlier checksum guard.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
CARGO_BUILD_JOBS=2 cargo test --lib \
  infrastructure::page::tests::impossible_entry_counts_are_rejected_before_collection_allocation
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --lib infrastructure::page::tests
  6 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
  passed

CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  219 tests passed; 0 failed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

This closes the impossible-count allocation path for fixed pages. It does not
replace the finite corruption-mutation matrix or close the external Linux,
filesystem power-loss, and pinned RocksDB gates.
