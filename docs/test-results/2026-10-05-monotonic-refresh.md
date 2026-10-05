# Monotonic refresh and missing WAL recovery

Source commit under test: `ab04f24` (`fix: reject refresh snapshot
regression`).
Equal-head follow-up: `a602740` (`fix: verify equal-head refresh
replacements`).
Main-generation coverage: `ad2041e` (`test: cover divergent main refresh`).

## Defect

A live buffered handle correctly rejected commits after its nonempty `.wal`
path was removed or renamed. `Database::refresh`, however, fell back to a full
reopen. WAL open semantics create a missing sidecar, so refresh could create an
empty WAL, select the older main-file checkpoint, and replace the handle's
newer in-memory durable snapshot with it.

The acknowledged records still existed through the old open descriptor, but
the public handle could silently move its transaction ID backwards. Ordinary
commit-path identity tests did not exercise refresh, and byte-identical WAL
replacement did not trigger the regression because the replacement retained
the same records.

## Corrected contract

- `Database::refresh` compares the candidate and currently installed
  transaction IDs before publication. A lower candidate returns
  `Error::WriteConflict` and leaves the current store and head unchanged.
- During buffered incremental refresh, an unchanged main checkpoint plus a
  missing or empty current WAL path cannot replace a known nonempty WAL. The
  adapter returns `WriteConflict` before `OpenOptions::create` can recreate the
  sidecar.
- A changed main checkpoint still falls back to normal recovery, preserving
  valid cross-process checkpoint and compaction transitions.
- Restoring the original renamed WAL makes refresh succeed at the same
  transaction ID; later writes remain available and rejected writes remain
  absent.
- If a reopened generation has the same transaction ID, refresh fully verifies
  both snapshots and requires identical entry counts and logical hashes before
  publication. A valid but divergent checksummed WAL or main generation is
  therefore rejected.
- Direct refresh on an unchanged main inode with no external WAL reuses the
  existing store and reads only current metadata, avoiding full verification
  on the ordinary path. Generation replacement and WAL recovery still reopen.

The application-layer monotonic check is storage-adapter independent and has a
unit test whose fake engine deliberately returns an older head. The Unix
integration test additionally proves that the WAL path stays absent after the
failed refresh and that the current in-memory read remains available.

## Verification

```text
cargo fmt --all -- --check
  passed

CARGO_TARGET_DIR=/tmp/wsldb-refresh-regression.<temporary> \
  cargo test --lib \
    refresh_rejects_a_transaction_regression_without_changing_the_snapshot \
    --all-features
  1 passed; 0 failed

CARGO_TARGET_DIR=/tmp/wsldb-refresh-regression.<temporary> \
  cargo test --test path_identity --all-features
  13 passed; 0 failed

CARGO_TARGET_DIR=/tmp/wsldb-refresh-regression.<temporary> \
  cargo test --test refresh --all-features
  5 passed; 0 failed

CARGO_TARGET_DIR=/tmp/wsldb-refresh-regression.<temporary> \
  cargo clippy --lib --test path_identity --test refresh \
    --all-features -- -D warnings
  passed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

git diff --check
  passed
```

The private Cargo target was removed after verification, and the repository
has no `target` directory. This deterministic regression test does not replace
physical power-loss validation. No benchmark, cache reset, reboot, shutdown,
sleep, raw-device operation, or power-removal test was run on the development
laptop.

The equal-head follow-up was verified separately:

```text
CARGO_TARGET_DIR=/tmp/wsldb-refresh-equivalence.<temporary> \
  cargo test --test path_identity --all-features
  14 passed; 0 failed

CARGO_TARGET_DIR=/tmp/wsldb-refresh-equivalence.<temporary> \
  cargo test --test refresh --all-features
  5 passed; 0 failed

CARGO_TARGET_DIR=/tmp/wsldb-refresh-equivalence.<temporary> \
  cargo test --test write_buffer --all-features
  16 passed; 0 failed

CARGO_TARGET_DIR=/tmp/wsldb-refresh-equivalence.<temporary> \
  cargo test --lib \
    refresh_rejects_a_transaction_regression_without_changing_the_snapshot \
    --all-features
  1 passed; 0 failed

CARGO_TARGET_DIR=/tmp/wsldb-refresh-equivalence.<temporary> \
  cargo clippy --lib --test path_identity --test refresh --test write_buffer \
    --all-features -- -D warnings
  passed
```

That private target was also removed; it used no database benchmark or system
cache operation.

The symmetric main-generation test then passed the focused path-identity suite
with 15 tests and the unchanged five-test refresh suite. Focused Clippy with
all features remained warning-free. Its 101.2 MiB private target was removed.
