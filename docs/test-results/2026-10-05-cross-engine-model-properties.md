# Cross-engine model-property coverage

Date: 2026-10-05

## Gap

The always-on suite compared direct and buffered mutations with independent
ordered models in separate tests. Direct mode did not combine that comparison
with historical-snapshot checks, while buffered mode used one fixed range.
Random bounded and open-ended range checks existed in the opt-in ten-million
operation soak, but not in the ordinary release suite shared by both engines.

## Coverage

`tests/model_properties.rs` now drives the direct copy-on-write and durable
buffered engines through the same deterministic model scenario. Across 1,152
mixed puts, updates, and deletes it checks:

- read-your-writes after every mutation;
- exact ordered contents and entry counts for every committed snapshot;
- immutability of the preceding snapshot after each commit and after a
  buffered checkpoint;
- random bounded, open-ended, reversed, and zero-limit ranges against an
  independent `BTreeMap` projection;
- empty, inline, and overflow-backed values;
- full verification, explicit buffered flush, and recovery after reopening
  without buffered options.

The test uses a fixed seed and completes in approximately half a second on the
recorded macOS host. It supplements rather than replaces the finite soak,
corruption-fuzz, crash-injection, and physical power-loss gates.

## Verification

All Cargo commands used at most two jobs and a private temporary target
directory that was deleted after completion.

```text
cargo test --test model_properties
  1 passed

cargo test --all-targets --all-features
  109 library tests passed
  158 integration tests passed
  2 power-loss probe tests passed
  5 benchmark entrypoints remained opt-in

cargo check --all-targets --all-features
  passed

cargo clippy --all-targets --all-features -- -D warnings
  passed

RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
  passed

cargo fmt --all -- --check
  passed

git diff --check
  passed

scripts/release-audit.sh
  passed
```

The complete executed suite contained 269 Rust tests. No benchmark workload,
opt-in soak body, corruption-fuzz body, cache reset, reboot, shutdown, sleep,
raw-device, or power operation ran.
