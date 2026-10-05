# Linux ext4 release evidence runner — 2026-10-04

## Subject

- WSLDB commit: `43699cb9831dce41a971c4934176b920241694c7`
- Runner: `scripts/run-linux-release-suite.sh`
- Intended host: Linux with an ext4 scratch filesystem
- Default build concurrency: two Cargo jobs
- Default free-space preflight: 5 GiB

## Protocol

The runner refuses a non-Linux host, dirty WSLDB checkout, non-ext4 scratch
filesystem, existing evidence path, insufficient free space, invalid resource
limits, or missing release tools. The evidence path must be outside the source
checkout.

It records the exact commit, clean status, kernel, Rust and Cargo versions,
CPU, memory, block devices, ext4 mount details, free-space threshold, build-job
limit, every command, every exit status, and GNU `time -v` output. It executes
the hygiene audit, format, check, Clippy, debug and release all-target suites,
rustdoc with warnings denied, and `cargo package`. A successful run retains the
packaged crate and its SHA-256.

Compilation and all test temporary files use one uniquely named directory
below the validated ext4 scratch parent. A guarded exit trap deletes only that
directory, including on failure; raw evidence remains in the requested output
directory.

## Local validation

The following non-Linux checks passed:

```text
bash -n scripts/run-linux-release-suite.sh
scripts/release-audit.sh
git diff --check
```

Direct invocation on macOS failed before inspecting the supplied paths or
creating an evidence directory, as required.

## Scope

This validates script structure, packaging, macOS rejection, and integration
with local release hygiene. The runner has not been executed on Linux/ext4 for
the current accepted source. It therefore does not close the Linux release
gate or replace the earlier Linux evidence for an older commit.
