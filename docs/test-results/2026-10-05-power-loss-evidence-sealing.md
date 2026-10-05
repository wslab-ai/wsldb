# Power-loss evidence sealing

Date: 2026-10-05

## Scope

The physical power-loss runner now has an explicit finalization step for a
completed evidence set:

```text
scripts/run-power-loss-cycle.sh seal EVIDENCE_DIR
```

Preparation pins the SHA-256 values of both the runner and the common evidence
hash writer. `run`, `verify`, and `seal` reject a changed runner. Sealing also
rejects a changed or non-executable hash writer, an empty or non-contiguous
cycle set, missing technical records, an unsuccessful verifier, changed
hardware or cut-attestation records, and any cycle that claims manual review is
complete.

On success, sealing records the cycle count and UTC time, retains
`release-gate-complete=no`, and publishes `evidence-sha256.txt` over every
regular evidence file. The manifest makes the directory immutable to further
runner operations. It proves bundle integrity, not completion of the physical
power-loss release matrix.

## Verification

The deterministic runner test constructs only synthetic state under a
temporary directory. It verifies a valid one-cycle seal, checks every manifest
entry, rejects a repeated seal, and proves that a sealed directory cannot arm
another cycle. Existing fail-closed cases still cover incomplete hardware,
unchanged boot identity, orderly writer exit, and incomplete cut attestation.

```text
bash -n scripts/run-power-loss-cycle.sh \
  scripts/run-power-loss-cycle.test.sh \
  scripts/write-evidence-hashes.sh

scripts/run-power-loss-cycle.sh --check-host
  power-loss runner host support is available for Darwin

scripts/run-power-loss-cycle.test.sh
  power-loss runner fail-closed tests passed

scripts/write-evidence-hashes.test.sh
  evidence hash tests passed

git diff --check
  passed
```

No database build, benchmark, cache reset, reboot, shutdown, sleep, power cut,
or other host power operation ran. No physical durability result is claimed;
the APFS, ext4, and XFS release gate remains open.
