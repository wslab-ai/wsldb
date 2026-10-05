# Release evidence integrity manifest

Date: 2026-10-05

## Risk

The short and sustained comparison runners recorded source commits and hashes
of executed binaries, but the completed evidence directories had no integrity
manifest. Raw logs, resource reports, size tables, or derived checkpoint rows
could therefore be changed after collection without a repository-provided
mechanism detecting the change. A partially failed run was also distinguishable
only by inspecting its contents and process status.

## Contract

The common `scripts/write-evidence-hashes.sh` writer creates
`evidence-sha256.txt` only after its caller has completed all requested work.
Before hashing, each benchmark runner appends these fields to `manifest.txt`:

- `runner_complete=yes`;
- `evidence_hashes_file=evidence-sha256.txt`.

The final file contains a path-sorted SHA-256 for every regular file under the
evidence directory except itself. This includes `manifest.txt`,
`commands.log`, raw standard output and error, `/usr/bin/time` reports, build
records, `sizes.csv`, and sustained `checkpoint-windows.csv`. Creation uses a
private temporary file and one final rename; an existing integrity manifest is
rejected rather than overwritten. Empty bundles and paths containing a newline
also fail without publishing a final manifest.

The short runner records hashes of `run-comparison.sh`, the shared contract,
and the evidence writer.
The sustained runner records hashes of `run-sustained-comparison.sh`, the
shared contract, `checkpoint-window-report.sh`, and the evidence writer. The
Linux ext4 release runner records its own hash and the writer hash, retains the
package SHA-256, appends `suite_complete=yes`, and then seals its manifest,
commands, step table, raw logs, resource reports, and `.crate` artifact through
the same implementation. The physical power-loss runner records its own hash
and the writer hash at preparation time. Its explicit `seal` operation checks
complete contiguous cycles, retained status and attestation hashes before
using the same writer; it leaves `release-gate-complete=no` for manual matrix
review. These complement the existing hashes of executed binaries and any
cold-cache reset helper.

A failed or interrupted run does not reach the final hash-manifest operation.
It remains useful diagnostic material, but cannot be mistaken for a completed
evidence bundle. Consumers verify the bundle from its root with
`sha256sum -c evidence-sha256.txt` on Linux or
`shasum -a 256 -c evidence-sha256.txt` on macOS.

## Verification

The dependency-free writer test covers exact hashes, deterministic relative
paths, repeated invocation, empty evidence, invalid filenames, and missing
directories. The benchmark fixture exercised both comparison runners. It
recomputed the hashes of manifests, command logs, and joined checkpoint CSV,
required exactly one matching record for each, and proved that a rejected
sustained result had no final `evidence-sha256.txt`.

```text
bash -n benchmarks/benchmark-contract.sh \
  benchmarks/run-comparison.sh \
  benchmarks/run-sustained-comparison.sh \
  benchmarks/benchmark-contract.test.sh \
  scripts/run-linux-release-suite.sh \
  scripts/write-evidence-hashes.sh \
  scripts/write-evidence-hashes.test.sh

scripts/write-evidence-hashes.test.sh
  evidence hash tests passed

benchmarks/benchmark-contract.test.sh
  benchmark contract tests passed

scripts/release-audit.sh
  benchmark contract tests passed
  power-loss runner fail-closed tests passed
  release hygiene audit passed

git diff --check
  passed
```

No benchmark workload, cache reset, reboot, shutdown, sleep, or power operation
ran. The power-loss seal test used only synthetic files in a temporary
directory. This change protects collected evidence; it does not supply the
still-open pinned RocksDB, cold-cache, Linux, or physical power-loss results.
