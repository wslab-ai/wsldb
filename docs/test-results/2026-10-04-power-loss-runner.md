# External power-loss runner verification — 2026-10-04

Source commit under test:
`944435952660f909bff47064c7a05e921d1312b9`.

Host scope: Darwin 26.6.2. This was a tooling-only verification on the
development host. No reboot, shutdown, power-off, physical power cut, raw
device write, probe build, database initialization, or `prepare`, `run`, or
`verify` cycle was performed. The real APFS, ext4, and XFS power-loss release
gate remains open and must run on isolated sacrificial test hosts.

The following bounded checks passed:

```text
bash -n scripts/run-power-loss-cycle.sh \
  scripts/run-power-loss-cycle.test.sh scripts/release-audit.sh
scripts/run-power-loss-cycle.sh --check-host
  power-loss runner host support is available for Darwin
scripts/run-power-loss-cycle.test.sh
  power-loss runner fail-closed tests passed
scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner host support is available for Darwin
  power-loss runner fail-closed tests passed
  macOS cache reset host support is available
  release hygiene audit passed
git diff --check
  passed
```

The self-test used only a temporary directory and empty fixture files. It
confirmed fail-closed rejection of same-mount preparation, incomplete hardware
metadata, unchanged boot identity, orderly writer exit, and incomplete
physical-cut attestation before probe verification. The temporary directory
was deleted by the test trap. The repository `target` directory was absent
after verification.
