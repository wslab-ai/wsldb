# Isolated full-verification harness

Date: 2026-10-04

## Purpose

The sustained W5/W6 adapters measure full verification only inside a long
mixed workload. That made a verification-only optimization require an
unnecessarily long rerun and made causal attribution difficult.

## Contract

`benchmarks/verification.rs` opens an explicitly selected existing database,
performs one warm-up and five measured full verifications by default, and emits
the complete `VerificationReport` identity beside every elapsed interval. It
fails if transaction ID, counts, depth, or logical SHA-256 changes between
runs. Open and WAL recovery occur outside the measured interval. The harness
does not claim a result by itself; external run order, cache state, resource
measurements, executable identity, hardware, and filesystem evidence remain
mandatory.

## Verification

Compilation and disabled-by-default execution evidence is recorded after the
short checks below. No large database or performance workload was run on the
development laptop for this change.

```text
cargo fmt --all -- --check
PASS

cargo check --bench verification --all-features
PASS

cargo test --bench verification --all-features
PASS: exited without running because WSLDB_RUN_VERIFY_BENCH was unset
```

An enabled smoke copied the committed decoder fixture into an isolated
temporary directory and requested one warm-up plus two measured runs. Both
runs returned transaction 2, 299 entries, 9 tree pages, 7 overflow pages,
depth 2, and logical SHA-256
`df3499d87f8adf4d9f76891ca8c96db2f5e4509c3a85ce4752384ca2f4656c77`.
The debug-build elapsed intervals were 12.618 and 12.880 milliseconds; they
only validate harness execution and are not performance evidence. The exact
temporary main and lock files were removed after the smoke.
