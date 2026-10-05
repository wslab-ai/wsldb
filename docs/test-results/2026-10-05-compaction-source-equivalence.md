# Compaction source-equivalence audit

Date: 2026-10-05

Implementation commits:

- `e3f6bd3` (`fix: prove compaction logical equivalence`);
- `19a0235` (`fix: verify compaction source structure`).

## Risk

The compacted destination was fully verified, but that alone did not prove it
contained the same logical snapshot as its source. The streaming compaction
cursor enforced ordered keys and the metadata entry count, but did not validate
branch separators, balanced depth, overlapping child ranges, or repeated page
references. A checksum-valid malformed source tree could therefore be rebuilt
into a valid but silently normalized destination.

## Contract

Both direct and checkpoint-plus-WAL compaction now use the authoritative full
tree traversal for their source stream. During that single pass WSLDB:

- validates page reachability, balanced depth, branch separators, child ranges,
  overflow chains, and the metadata entry count;
- computes the canonical ordered key/value SHA-256 and entry count while
  feeding the bottom-up loader;
- fully verifies the compacted destination; and
- requires the destination entry count and logical hash to equal the source
  stream before truncate, final sync, or publication.

Buffered backup uses the same layered rebuild and therefore inherits the same
pre-publication contract. The verifier transfers each reconstructed value to
the compaction visitor, avoiding a second large-value copy. Maintenance still
stages at most 4 MiB or 32,768 entries, while full structural validation keeps
one fallibly allocated reachability record per source page.

The causal unit test supplies a two-leaf in-memory tree whose checksum-level
nodes are readable but whose branch separator disagrees with the right-child
minimum. Direct and layered compaction both return `Error::Corrupt`, and neither
publishes its target. A separate guard test rejects independently mismatched
entry counts and logical hashes.

## Verification

The complete local audit used a private temporary Cargo target and
`CARGO_BUILD_JOBS=2`; the target was deleted afterwards. The commands were:

```text
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
scripts/release-audit.sh
```

Results:

- 99 library tests passed;
- 152 integration and example tests passed, including crash injection,
  injected I/O failure, compaction races, backup races, and multiprocess
  writers;
- all five benchmark entrypoints remained behind their explicit opt-in gates,
  so no performance workload ran on the development laptop;
- check, Clippy, formatting, diff, and release hygiene completed without a
  warning or error;
- the repository `target` directory and private temporary targets were absent
  after cleanup.

This audit proves source/destination equivalence and fail-closed structural
handling for the tested implementation. It does not replace the still-open
real power-loss, current Linux/ext4, or frozen RocksDB comparison gates.
