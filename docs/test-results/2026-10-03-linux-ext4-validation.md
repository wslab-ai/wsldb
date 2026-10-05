# Linux ext4 validation — 2026-10-03

## Subject

- Current all-feature commit: `5120b04eb79efb1e6809691ac7c2187c4c24f58d`
- Range-model commit: `d7e1ddf7f2f56a894f607529ee4f60a0f4c4ae7d`
- Corruption-gate commit: `df81a6927d5a4b2c5009a40beda7999d7dcec9bd`
- The range-model commit predates the explicit group API. Its tree and range
  workload is unchanged; current all-feature and corruption runs cover the new
  WAL path.
- Image: official `rust:1.89-alpine` at
  `sha256:4b800f2e72e04be908e5f634c504c741bd943b763d1d8ad7b096cc340e1b5b46`
- Kernel: Linux 6.8.0-100-generic, arm64, inside a Colima virtual machine
- Userspace: Alpine Linux 3.22.1, musl
- Filesystem: ext4 guest root mounted `rw,relatime,discard,errors=remount-ro,commit=30`
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`
- Repository state before every counted run: clean

The source tree was mounted read-only. Compilation used an executable tmpfs,
and test databases used an isolated directory on the Linux guest's ext4 root.

## All-feature suite

```text
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/build TMPDIR=/work \
  cargo test --all-targets --all-features
```

The current commit passed all 123 tests. This includes direct and buffered
process-crash boundaries, short writes, storage-full and sync failures,
maintenance publication failures, active generation replacement, refresh,
backup, compaction, direct/buffered multi-process writer stress, and committed
database/WAL compatibility fixtures decoded through the public API. The suite
also covers explicit group ordering, shared-sync accounting, recovery,
checkpoint, partial writes, sync ambiguity, stale handles, and process crash.

The suite also verified the test-harness cleanup regression. Crash workers had
previously left main-file replacement artifacts in the system temporary
directory. `TestFile` now removes main-file and WAL replacement names, and the
isolated ext4 test directory was empty after the complete suite.

Raw local evidence:
`wsldb-benchmark-evidence/2026-10-03-linux-ext4-all-features-5120b04-clean`.

## Ten-million-operation model gate

```text
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/build TMPDIR=/work \
WSLDB_RUN_SOAK=1 \
  cargo test --release --test soak \
  model_based_ten_million_operation_soak -- --exact --nocapture
```

- Status: passed
- Model operations: 10,000,000
- Durable batch commits: 10,000
- Full verify, deterministic point/range sample, and reopen checkpoints: 100
- Sampled point reads: 100,000
- Model-compared range reads: 10,000
- Verified active-generation compactions: 33
- Largest generation before compaction: 770 MiB
- Final file/live data: 499 MiB / approximately 5 MiB
- Final full ordered comparison with the independent model: exact
- Test time: 536.44 seconds
- Whole command time, including release compilation: 547.46 seconds

An initial run on a macOS-hosted virtiofs bind mount exhausted that mount at
1.6 million operations and is retained as failed environment evidence. It is
not counted as a database pass or failure. The ext4 rerun kept the workload,
seed, ten-million-operation count, and 512 MiB compaction threshold unchanged.

Raw local evidence:
`wsldb-benchmark-evidence/2026-10-03-linux-ext4-model-soak-10m-ranges-clean`.

## Finite corruption gate

```text
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/build TMPDIR=/work \
WSLDB_RUN_CORRUPTION_FUZZ=1 \
WSLDB_CORRUPTION_FUZZ_ITERATIONS=100000 \
  cargo test --release --test corruption_fuzz -- --nocapture
```

- Status: passed
- Main-file mutations: 100,000
- WAL mutations: 100,000
- Total mutations: 200,000
- Panics, hangs, or unknown accepted logical states: zero
- Test time: 137.83 seconds
- Whole command time, including release compilation: 177.70 seconds

The WAL baseline contains two consecutive records written by one explicit
group commit. Accepted recovery states are limited to the checkpoint, the
complete first-record prefix, or the complete group.

Raw local evidence:
`wsldb-benchmark-evidence/2026-10-03-linux-ext4-corruption-fuzz-100k-df81a69-clean`.

## Scope

These results close the finite model, corruption, and current all-feature gates
for this virtualized Linux/ext4 environment. They do not prove bare-metal
behavior, actual kernel or machine power loss, XFS, Windows/NTFS behavior, or
comparative production performance.
