# Packed-leaf verification preregistration

Date: 2026-10-04. This document was committed before the candidate
implementation or its candidate measurements.

## Observation

The current full verifier materializes every packed leaf as a `Vec<LeafEntry>`.
That copies every key and inline value before the verifier immediately copies
the inline value again to feed the logical hash. The persisted page still has
to be decoded and checksummed, but these per-entry allocations are not part of
the integrity contract.

The isolated W7 harness ran against a post-prefix-WAL fixture containing
1,000,000 entries, 16,578 tree pages, no overflow pages, and tree depth 3. One
warm-up followed by five measured runs produced 836.283, 832.495, 822.941,
829.860, and 831.357 milliseconds. The median baseline is 831.357 milliseconds.
Every run returned transaction 100 and logical SHA-256
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.

## Candidate

Verify packed leaves directly. Construct at most the current key from its
shared prefix and suffix, pass inline values to the visitor by reference, and
materialize only overflow values because their bytes span multiple pages.
Ordinary decoded leaves and the persisted bytes remain unchanged.

This candidate targets the separate W7 verification deficit. It is not
expected to improve W6 mutation latency, which remains dominated by durable
WAL synchronization.

## Acceptance gate

The candidate is accepted only if:

- the same fixture returns the identical transaction, entry count, structural
  page counts, depth, and logical SHA-256 in every run;
- median full-verification latency improves by at least 8% in the same one
  warm-up plus five measured-run process;
- focused packed-leaf, overflow, corruption, and verification tests pass;
- formatting, `cargo check --all-targets --all-features`, Clippy with warnings
  denied, and `git diff --check` pass.

The fixture is diagnostic evidence on one warm APFS host. It does not replace
the repeated cross-engine W7 release gate.

## Result

Rejected. The candidate returned the identical transaction, entry count, page
counts, depth, and logical SHA-256 in all five measured runs. Its latencies
were 800.322, 800.118, 816.285, 805.245, and 790.158 milliseconds, for a
median of 800.322 milliseconds. That is 3.73% below the 831.357-millisecond
baseline, but it misses the preregistered 8% acceptance threshold.

The candidate implementation was removed. This confirms that packed-leaf
materialization contributes to verification cost, but is not the dominant W7
bottleneck on this fixture.
