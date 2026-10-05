# Evidence seal verification

Date: 2026-10-05

## Scope

Release, comparison, sustained-comparison, and power-loss evidence runners now
share a fail-closed verifier for their final `evidence-sha256.txt` seal. The
verifier checks every recorded SHA-256 value and requires the manifest to cover
the exact set of retained regular files. It rejects missing and unlisted files,
duplicate records, unsafe relative paths, symbolic links, non-regular entries,
and an empty manifest.

Each runner records the verifier's SHA-256 before creating the seal. The
power-loss protocol additionally pins that identity at preparation and checks
it again when sealing. A runner removes `evidence-sha256.txt` if its immediate
self-verification fails, so failed or interrupted output cannot retain an
accepted completion marker.

## Focused verification

The verifier test exercised a valid nested evidence set, a filename containing
spaces, changed content, a missing listed file, an extra unlisted file, a
duplicate record, an unsafe parent path, a symbolic link, a FIFO, an empty
manifest, and a missing evidence directory.

```text
$ scripts/verify-evidence-hashes.test.sh
evidence hash verifier tests passed
```

The existing integration contracts exercised the common verifier through both
benchmark runners and the power-loss sealing path.

```text
$ scripts/run-power-loss-cycle.test.sh
power-loss runner fail-closed tests passed

$ benchmarks/benchmark-contract.test.sh
benchmark contract tests passed
```

The repository release audit reran all shell contracts, package membership
checks, and host-safe protocol checks after the integration.

```text
$ scripts/release-audit.sh
benchmark contract tests passed
power-loss runner host support is available for Darwin
power-loss runner fail-closed tests passed
evidence hash verifier tests passed
evidence hash tests passed
macOS cache reset host support is available
release hygiene audit passed
```

No reboot, shutdown, sleep, cache reset, destructive power operation, or
long-running workload was performed by this verification.
