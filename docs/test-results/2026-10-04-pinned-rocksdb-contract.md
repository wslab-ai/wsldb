# Pinned RocksDB benchmark contract — 2026-10-04

## Subject

- WSLDB commit: `aa9ab4fb04e97c3c55f01202e98c03226c234eba`
- Frozen RocksDB release: `v11.8.1`
- Frozen RocksDB commit:
  `abeebd9630f11bd08c28b7bd43c7bdfc62050654`
- Commit lookup: official `facebook/rocksdb` Git tag namespace
- RocksDB headers used for adapter syntax validation: the clean checkout at
  the frozen commit

## Contract

Both comparison runners now default to strict release mode. Before creating an
evidence directory or starting a workload, they require:

- a clean WSLDB checkout;
- a clean RocksDB checkout at the exact frozen commit;
- an executed RocksDB benchmark that reports version `11.8.1` exactly;
- for direct W7 evidence, an open adapter reporting the same linked version;
- a WSLDB benchmark built by the runner from the recorded checkout rather than
  an externally supplied binary.

Diagnostic mode remains available explicitly. Its manifest records that the
source contract is not satisfied. A strict individual run records that the
source contract is satisfied but that the complete release matrix is not; this
prevents one narrow result from being presented as the whole performance gate.

The shared shell self-test covers the passing contract and rejects a dirty
WSLDB tree, dirty RocksDB tree, wrong RocksDB commit, wrong binary version,
missing WSLDB checkout, missing RocksDB checkout, and invalid mode.
It also executes both complete runner front ends in zero-work diagnostic mode
with stub binaries, then validates their source-scope fields and executable
SHA-256 fields. This catches integration failures that `bash -n` cannot find.

## Verification

The following commands passed on the clean subject commit:

```text
scripts/release-audit.sh
c++ -std=c++20 -Wall -Wextra -Wpedantic -Werror -fsyntax-only \
  -I/path/to/rocksdb-v11.8.1/include -I/path/to/rocksdb-v11.8.1 \
  benchmarks/rocksdb_open_bench.cc \
  benchmarks/rocksdb_sustained_bench.cc
cargo fmt --all -- --check
git diff --check
```

The release audit ran the benchmark-contract self-test and confirmed that all
contract scripts and adapter sources are included in the Cargo package.

No timed workload was run for this change. This evidence closes an evidence
provenance hole; it does not close the frozen RocksDB performance matrix.
