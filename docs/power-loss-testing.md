# Real power-loss testing

Process aborts prove recovery logic at instrumented boundaries, but they do not
prove device-cache, filesystem, controller, or mount-option durability. The
production gate therefore requires short real power-removal cycles on APFS,
ext4, and XFS with the exact hardware and mount assumptions recorded.

Run those cycles only on an isolated, sacrificial test host. Never run them on
an active development laptop or a machine containing data that is not backed
up. The supplied runner never invokes `reboot`, `shutdown`, `poweroff`, or
`halt`; an operator controls the external power cut and subsequent boot.

`examples/power_loss_probe.rs` is a resumable two-file probe. Each database
transaction creates one immutable numbered marker and updates a redundant
logical-head value. Only after `Database::commit` returns does the probe append
the same number plus its complement to an oracle and `sync_all` that oracle.
It then prints `ACK N`. The next database commit does not begin before that
oracle sync.

The oracle must live on an independently durable device or remote/serial
collector. Keeping it beside the database cannot prove that one shared volatile
cache did not lose both acknowledgements. Capture the printed `ACK` stream on
the independent collector as an additional audit trail.

## One cycle

The evidence runner builds and pins the probe, rejects a dirty checkout,
requires both the oracle and retained evidence to use mount sources different
from the tested database, records the boot identity, and fails closed when
required metadata is missing. Check host prerequisites without creating files
or building code:

```sh
scripts/run-power-loss-cycle.sh --check-host
```

On the isolated test host, prepare a new direct or buffered evidence set:

```sh
scripts/run-power-loss-cycle.sh prepare buffered \
  /tested-filesystem/wsldb-power.wsldb \
  /independent-device/wsldb-power.oracle \
  /retained-evidence/wsldb-buffered-run
```

Fill the generated `hardware.txt`, then arm the next cycle:

```sh
scripts/run-power-loss-cycle.sh run \
  /retained-evidence/wsldb-buffered-run
```

The writer is intentionally unbounded. Remove power externally at a
nondeterministic point. If the process exits normally, the runner retains the
cycle as failed evidence. After the host boots again, fill the generated
`cut-attestation.txt` and verify before starting another cycle:

```sh
scripts/run-power-loss-cycle.sh verify \
  /retained-evidence/wsldb-buffered-run
```

Verification refuses an unchanged boot identity, a changed database mount, an
orderly writer exit, a modified probe, or incomplete attestation. A technical
pass remains marked `manual-review-required=yes` and
`release-gate-complete=no`; it does not replace review of the complete matrix.

After the planned cycles for one evidence set have been technically verified,
seal that set before review or transfer:

```sh
scripts/run-power-loss-cycle.sh seal \
  /retained-evidence/wsldb-buffered-run
```

Sealing checks the pinned runner, hash-writer, and hash-verifier identities,
complete and contiguous cycle numbering, required records, successful probe
status, and the recorded hardware and cut-attestation hashes. It then writes
`evidence-sha256.txt` over every retained regular file and verifies exact
file-set coverage before reporting success. A verification failure removes the
seal. A successful seal prevents another `run`, `verify`, or `seal` operation
from modifying that evidence directory. After transfer, run
`scripts/verify-evidence-hashes.sh /retained-evidence/wsldb-buffered-run`
before accepting it. Sealing deliberately keeps `release-gate-complete=no`;
the complete filesystem/mode/cut-point matrix still requires manual review.

The equivalent manual protocol is documented below for auditability.

Build once, then initialize a fresh direct or buffered probe. The paths must
not exist:

```sh
cargo build --release --example power_loss_probe
target/release/examples/power_loss_probe \
  init buffered /tested-filesystem/wsldb-power.wsldb \
  /independent-device/wsldb-power.oracle
```

Start an unbounded writer and remove machine power at a nondeterministic point;
do not send the process a signal or perform an orderly shutdown:

```sh
target/release/examples/power_loss_probe \
  run buffered /tested-filesystem/wsldb-power.wsldb \
  /independent-device/wsldb-power.oracle
```

After reboot and filesystem recovery, verify before resuming:

```sh
target/release/examples/power_loss_probe \
  verify buffered /tested-filesystem/wsldb-power.wsldb \
  /independent-device/wsldb-power.oracle
```

Verification performs WSLDB's complete structural/value check, requires every
database marker from one through its logical head with no gaps or extras, and
requires the database head to equal the last complete oracle record or lead it
by exactly one. A one-record lead is the transaction whose database sync
completed while its oracle append was interrupted. Falling behind the oracle
means an acknowledged commit was lost; leading by more than one means the
serialized protocol was violated or multiple durable oracle records vanished.

`run` safely truncates only an incomplete oracle tail and records a previously
verified one-record database lead as `RECOVERED` before starting the next
cycle. Pass a final numeric argument to bound a smoke run. Buffered mode uses a
16 KiB checkpoint threshold by default so short cycles exercise WAL and
checkpoint transitions; `WSLDB_POWER_LOSS_BUFFER_BYTES` overrides it. Repeat
the complete matrix in both `direct` and `buffered` modes.

## Evidence record

For every cycle retain:

- WSLDB commit and executable SHA-256;
- OS, kernel, filesystem, filesystem-tool version, mount options, block-device
  model/firmware, controller, cache and power-protection settings;
- whether the oracle used a separate device, host, or serial collector;
- pre-cut `ACK` output, boot/recovery logs, verifier output, database/WAL/oracle
  sizes and SHA-256 values;
- the method and approximate timing of physical power removal;
- pass/fail classification without discarding failed cycles.

Retain the final `evidence-sha256.txt` and run the common evidence verifier
before accepting or archiving the set. A directory without that manifest is
unfinished diagnostic material, not completed release evidence.

Run multiple cuts during direct page commits, buffered WAL appends, automatic
checkpoint preparation/publication, and reopen. The gate is not satisfied by a
single successful cut, VM termination, process kill, or filesystem unmount.
