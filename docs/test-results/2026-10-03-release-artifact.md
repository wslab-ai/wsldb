# Local release-artifact audit — 2026-10-03

## Scope

This local audit checks that the Rust package can be assembled and verified
from a clean repository without depending on files outside the package. It does
not replace the remaining Linux release checklist, filesystem power-loss, or
finite sustained-performance gates. Windows is outside the initial official
support matrix.

## Subject

- WSLDB commit: `bc1f26e80800dd9dd2b80e38d2ee25c68f4c5efc`
- Package name: `wsldb`
- Cargo package version: `1.0.0`
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Host: Apple M1 Max, macOS 26.6.2, arm64 APFS

## Results

`cargo package` ran without `--allow-dirty`, assembled 128 files totaling
794.8 KiB, produced a 157.5 KiB compressed crate, unpacked it, and compiled the
unpacked package successfully. Its generated VCS metadata names the exact clean
commit above. The packaged crate SHA-256 is
`3b62446b31c28685c9a09f8288218780f180a0b06f9d573adb73d28bd197be3d`.

The unpacked package contains no case-insensitive match for the removed legacy
database name, removed attribution file, an internally numbered file-format
document, or an internal pre-release version. It includes all three committed
database/WAL compatibility fixtures and their public-API decoder test. The
package metadata intentionally keeps `publish = false`; this audit does not
publish to crates.io or any other registry.

The exact clean commit also passed:

- three README documentation tests;
- `cargo doc --no-deps --all-features` with rustdoc warnings denied;
- all-target/all-feature `cargo check` and Clippy with warnings denied;
- `git diff --check`.

The optimized all-target/all-feature test suite for the same implementation is
[recorded separately](2026-10-03-release-profile-suite.md).

The package size is already small enough that excluding tests, benchmark
adapters, or reproducibility reports would not improve runtime performance.
They remain in the local artifact so the implementation and its evidence can be
audited together.
