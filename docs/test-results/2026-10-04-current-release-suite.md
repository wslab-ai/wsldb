# Current local release suite

Date: 2026-10-04. Host: Apple M1 Max, macOS 26.6.2, APFS.

## Subject

- Commit: `b8bccd71beccf9485a9dfc681b180c5a0d33146a`.
- Repository state: clean before every command.
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`.
- Cargo: `cargo 1.89.0 (c24e10642 2025-06-23)`.

## Verification

The following commands completed successfully:

```text
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo test --release --all-targets --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --all-features --no-deps
cargo package --list
cargo package --allow-dirty
git diff --check
```

Both debug and optimized release suites passed 174 tests with zero failures.
That count includes 58 library tests and all integration contracts for direct
and buffered recovery, committed format fixtures, process-crash injection,
partial and failed I/O, ambiguous sync outcomes, backup/compaction publication,
checkpoint races, stale handles, concurrent readers, and multi-process writers.
The opt-in ten-million-operation and finite 200,000-mutation tests remain
separate evidence; their ordinary entry points correctly no-op without their
environment gates.

Rustdoc completed with every warning denied. `cargo package` selected 186
files, produced a 228,152-byte crate archive, unpacked it, and compiled the
independent packaged source successfully. The archive SHA-256 is
`183f6afae95e790a94637c5b79a4c00df0675d1b5e8b325776ff206865f5d49f`.
The exact `cargo package --list` output SHA-256 is
`a3dc32beb204f00b652ef8ab65174b4cf58f6c2d64b79e1af730e3bae7680365`.

This closes the current local compile, lint, documentation, package, and
ordinary release-suite checks. It does not replace the open current-format
Linux rerun, real power-loss tests, or long-duration performance gates.
