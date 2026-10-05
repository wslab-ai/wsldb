# Runner input snapshot validation

Date: 2026-10-05

## Scope

Release evidence previously recorded source commits and executable hashes only
at startup. A source checkout, runner helper, adapter, benchmark executable, or
cold-cache hook changed during a long run could therefore leave a sealed set
whose initial manifest no longer described every process that produced it.

The comparison runners now capture each input hash once, use that exact value
in the manifest, and revalidate the file before sealing. Release-mode runs also
require both source checkouts to retain their initial commits and remain clean.
They compile WSLDB from a private detached clone of the recorded commit inside
runner scratch space, preventing working-tree edits from entering a build in
progress. The Linux release runner executes its audit, tests, documentation,
and packaging from the same kind of isolated snapshot and applies the input
rule to its original checkout, runner, hash writer, and hash verifier.
Successful manifests contain
`runner_inputs_revalidated=yes`; a mismatch exits before the completion marker
or evidence seal is created.

Cargo builds and release checks now use `--locked`, so a missing or stale lock
file fails instead of being repaired implicitly during evidence collection.

## Verification

The benchmark contract test covers an unchanged file, changed file, missing
file, unchanged clean source, dirty source, changed source commit, creation of
an isolated detached snapshot, and the explicit diagnostic-mode exemption. It
also runs both comparison runners with stub executables and requires their
sealed manifests to contain the input revalidation marker.

```text
$ benchmarks/benchmark-contract.test.sh
benchmark contract tests passed
```

The shell syntax checks and release hygiene audit cover the Linux runner and
the locked metadata/package-list commands without starting a benchmark or
release build.

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

No reboot, shutdown, sleep, cache reset, power action, or long-running workload
was used for this validation.
