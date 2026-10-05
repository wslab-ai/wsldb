# WAL mutation-count verification — 2026-10-05

Source commit under test:
`d03f81e4dbd5f598eaed63acc4c7dd9315efbbe7`
(`fix: bound wal mutation decoding`).

## Risk and change

An ordinary transaction or snapshot WAL frame stores its mutation count as a
`u32`. A checksummed but structurally corrupt frame could advertise billions of
mutations in a small payload. The decoder would eventually reject the
truncated payload, but only after entering a loop controlled by the untrusted
count.

The decoder now proves before entering that loop that the advertised count can
fit in the payload. Every legacy mutation needs at least two two-byte key
lengths and one eight-byte value-length field, so a count greater than
`payload_len / 12` is impossible. This lower bound deliberately excludes key
and value bytes and therefore cannot reject a valid frame. Compact group WAL
frames already apply corresponding transaction- and mutation-count bounds.
The persisted format and public API are unchanged.

The focused mutation test starts with a valid record, replaces its mutation
count with `u32::MAX`, recomputes the payload, header, and complete-record
checksums, and requires the specific structural count-bound error. Recomputing
the checksums proves the failure is not supplied by an earlier checksum guard.

## Verification

The build was bounded to two Cargo jobs. Benchmark and opt-in soak modes were
not enabled. No reboot, shutdown, power-cut, or raw-device operation occurred.

```text
CARGO_BUILD_JOBS=2 cargo test --lib \
  infrastructure::wal::tests::record_rejects_a_mutation_count_larger_than_its_payload
  1 passed; 0 failed

CARGO_BUILD_JOBS=2 cargo test --all-targets --all-features
  220 tests passed; 0 failed

CARGO_BUILD_JOBS=2 cargo check --all-targets --all-features
CARGO_BUILD_JOBS=2 cargo clippy --all-targets --all-features -- -D warnings
  passed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

cargo fmt --all -- --check
git diff --check
  passed
```

This closes the impossible-count loop bound for ordinary WAL records. It does
not replace the finite corruption-mutation matrix or close the external Linux,
filesystem power-loss, and pinned RocksDB gates.
