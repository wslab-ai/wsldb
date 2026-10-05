# Local release checklist

WSLDB does not use repository CI. Release validation is performed explicitly
on the supported macOS and Linux hosts, and evidence is committed under
`docs/test-results`. Windows remains best-effort and is not a release gate.

## Fast local audit

Run the deterministic hygiene audit from a clean checkout:

```sh
scripts/release-audit.sh
```

It validates package identity and licensing, keeps publishing disabled until
the release decision, rejects legacy names and unwanted release artifacts,
checks the exact unversioned format-fixture and public API compile-contract
SHA-256 values, enforces source-size limits, inspects the Cargo package file
list including those contracts, and runs `git diff --check`. It does not run
correctness tests or make a production readiness claim.

## Rust release checks

Run on both supported operating systems:

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo test --locked --release --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
cargo package --locked
```

The package must compile from its unpacked archive. Format fixtures, public API
contract tests, crash injection, I/O fault injection, model tests, and
multi-process tests are part of the all-target suites; their coverage still
requires review rather than relying only on the aggregate exit code.

On the Linux release host, run the same checklist through the evidence runner:

```sh
scripts/run-linux-release-suite.sh \
  /path/on/ext4/scratch-parent /new/evidence/directory
```

It refuses a dirty checkout, non-Linux host, non-ext4 scratch filesystem,
existing evidence directory, or less than 5 GiB of scratch space. It defaults
to two Cargo build jobs, records commands, exit codes, resource measurements,
kernel/toolchain/mount metadata, retains the verified `.crate` artifact and
SHA-256, and deletes only its own temporary target after success or failure. A
successful suite hashes its runner, the common evidence writer, and the common
verifier, then seals the manifest, per-step logs, resource reports, command
log, step table, and packaged crate in `evidence-sha256.txt`. Before reporting
success it proves that the source commit, isolated detached build snapshot,
clean checkout, and runner inputs did not change during the suite, then
verifies every digest and exact file-set coverage with
`scripts/verify-evidence-hashes.sh /new/evidence/directory`. Run the same
command again after transfer and before archival. A failed suite deliberately
removes or never creates the final integrity manifest.
`WSLDB_LINUX_RELEASE_BUILD_JOBS` and `WSLDB_LINUX_RELEASE_MINIMUM_KIB` may raise
or lower resource bounds, but any override is recorded in the manifest.

## External gates

Before the first release, attach current evidence for:

- the finite model soak and corruption mutation matrix on macOS/APFS and
  Linux/ext4;
- real power-loss recovery on APFS, ext4, and XFS with hardware and mount
  assumptions recorded using the independent-oracle protocol in
  [Real power-loss testing](power-loss-testing.md). Run
  `scripts/run-power-loss-cycle.sh` only on an isolated sacrificial test host;
  it never controls host power, and no development laptop run counts as
  evidence. After all planned cycles in a set pass technical verification,
  use its `seal` operation and verify the resulting directory with
  `scripts/verify-evidence-hashes.sh /retained/evidence/directory`;
  sealing preserves `release-gate-complete=no` until the entire matrix receives
  manual review;
- the frozen RocksDB workload matrix, including warm/cold cache, dataset sizes,
  resource use, latency distributions, and exact executable revisions. The
  comparison runners must remain in their default release mode, which rejects
  dirty source trees, any RocksDB commit other than the official `v11.8.1`
  commit, and adapters linked against another reported version. Cold-cache
  evidence must include the reset executable hash, every reset invocation and
  output, privilege setup, and platform-specific proof that the tested files
  were evicted. The supplied macOS and Linux helpers affect the global host
  cache and therefore run only on an isolated benchmark machine;
- stable file-format fixtures and the external-consumer API compile contract;
- backup restore, active compaction, refresh, and multi-process recovery.

Only after every gate in `production-readiness.md` passes may the documentation
drop its experimental warning, publishing be enabled, and the stable format and
API be declared frozen.
