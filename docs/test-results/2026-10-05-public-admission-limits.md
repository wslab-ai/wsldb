# Public mutation admission limits

Date: 2026-10-05

## API finding

The pre-release stability policy says resource limits needed for admission
control belong to the public API. The maximum explicit group size was exported,
but callers still had to duplicate the format's 1,024-byte key limit and
64 MiB value limit or discover rejection only while building a mutation.

## Change

The crate root now exports `MAX_KEY_BYTES` and `MAX_VALUE_BYTES`. The constants
are the same definitions used by `WriteTransaction` and `WriteBatch`
validation; no second copy of either limit was introduced. Mutation rustdoc and
the API-stability policy link to the exported values.

The external-consumer contract pins both names, types, and current values. Its
deliberately updated SHA-256 is:

```text
5baf6b89aa16270b750e9044191e369599099c19d8269b4ac943317a2e6b24e6
```

The release audit requires that exact contract hash. This is a compatible,
explicit pre-release addition; the prior baseline remains historical evidence.

## Boundary coverage

A focused unit test proves the key and value maxima are inclusive and that one
byte above either exported limit returns `Error::InvalidInput`. Value-length
coverage uses a length seam and does not allocate a 64 MiB test value.

## Verification

All Cargo commands used at most two jobs and private temporary target
directories that were deleted after completion.

```text
cargo test --all-features \
  application::mutation::tests::exported_admission_limits_are_inclusive
  1 passed

cargo test --all-features --test public_api
  3 passed

cargo test --all-targets --all-features
  109 library tests passed
  157 integration tests passed
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

The complete executed suite contained 268 Rust tests. No benchmark workload,
cache reset, reboot, shutdown, sleep, raw-device, or power operation ran.
