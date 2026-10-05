# WAL-handoff release audit

Date: 2026-10-04. Host: Apple M1 Max, macOS 26.6.2, APFS. Audited source
commit: `8425bbc818dbf1ab6274d210b9f54b987b6be52a`.

This audit validates the accepted bounded-memory maintenance and WAL-handoff
implementation as a local macOS release candidate. It does not declare WSLDB
production-ready and does not replace Linux or real power-loss evidence.

## Results

- `cargo fmt --all -- --check`: passed.
- `cargo check --all-targets --all-features`: passed without warnings.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --release --all-targets --all-features`: all 195 tests passed,
  including format fixtures, external API compilation, corruption mutation,
  process-crash, injected-I/O, multi-process, backup, checkpoint, WAL-handoff,
  compaction, and refresh coverage. Opt-in long harnesses remained disabled.
- The complete debug all-target suite passed during candidate validation; the
  exact release source then reran the affected debug compaction, write-buffer,
  maintenance-failure, and cleanup suites after the final test-only cleanup.
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`: passed.
- `cargo package`: packaged 231 files, 1.3 MiB uncompressed and 295.3 KiB
  compressed, then compiled the unpacked `wsldb 1.0.0` archive successfully.
- `scripts/release-audit.sh`: passed package identity, MIT license,
  publish-disabled state, legacy-name exclusion, source and Markdown size
  limits, package contents, and whitespace validation.
- `git diff --check`: passed.

The current format fixtures retained their committed unversioned names and
SHA-256 values. No internal file-format version was introduced. Cargo remains
at `1.0.0`, while `publish = false`, the README production warning, and the
unreleased format/API status remain in force until every external release gate
passes.

## Remaining external gates

- real power-loss recovery with recorded hardware and mount assumptions on
  APFS, ext4, and XFS;
- a current Linux release-suite repetition for the accepted source;
- the complete pinned RocksDB 11.8.1 workload matrix, including cold cache,
  resource accounting, and the sequential W6 mutation-tail requirement;
- the final explicit format/API freeze decision after those gates pass.
