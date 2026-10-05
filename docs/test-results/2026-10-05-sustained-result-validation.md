# Sustained result validation

Source commit under test: `0ebfaf1` (`fix: validate sustained benchmark
evidence`).

## Risk

The sustained comparison previously compared final fields only when an
operation limit was enabled. A duration-based process with a missing,
duplicated, or malformed `sustained_complete` line could therefore leave an
apparently complete evidence directory. Setup success also established that
both engines could open their databases, but the runner did not independently
prove that their initial logical datasets were identical before timing.

## Contract

- WSLDB load output now includes the exact entry count and logical SHA-256
  already computed by its full operational verification.
- Before timing, the runner requires exactly one `load_complete` record from
  each adapter, the configured entry count, valid lowercase 64-hex hashes, and
  equal hashes.
- Every measured or warm-up process, including duration-based runs, must emit
  exactly one `sustained_complete` record with a positive operation count, the
  configured entry count, and a valid logical SHA-256.
- Operation-limited conformance additionally requires both operation counts to
  equal the requested limit and both final logical hashes to match.
- Any absent or duplicate record, absent field, malformed count/hash, wrong
  entry count, setup mismatch, or operation-limited final mismatch fails the
  runner.

This validation happens outside the active workload interval and does not
change the timed operation stream.

## Verification

The executable-stub contract covers a valid run and explicit failure cases for
missing, duplicate, and malformed completion records plus unequal setup
hashes.

```text
bash -n benchmarks/run-sustained-comparison.sh \
  benchmarks/benchmark-contract.test.sh
bash benchmarks/benchmark-contract.test.sh
  benchmark contract tests passed

cargo fmt --all -- --check
  passed

CARGO_TARGET_DIR=/tmp/wsldb-bench-check.<temporary> \
  cargo check --bench wsldb
  Finished `dev` profile

CARGO_TARGET_DIR=/tmp/wsldb-bench-clippy.<temporary> \
  cargo clippy --bench wsldb -- -D warnings
  Finished `dev` profile

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

git diff --check
  passed
```

Both temporary Cargo target directories were removed immediately after the
checks, and the repository has no `target` directory. No database performance
workload, cache reset, reboot, shutdown, sleep, raw-device operation, or
physical power-loss test was run on the development laptop.

This proves runner integrity, not WSLDB performance or release readiness. The
pinned repeated W1-W7 comparison, current Linux/ext4 suite, supported-host
cold-cache measurements, and physical power-loss gates remain open.
