# Power-loss probe smoke validation

Date: 2026-10-04. Host: Apple M1 Max, macOS 26.6.2, APFS.
Validated source commit: `3283a9cb698dd5d3c0d210cedfa461780d194bd1`.

This result validates the deterministic probe mechanics only. It does not
contain a physical power removal and therefore does not satisfy the production
power-loss gate.

The new `examples/power_loss_probe.rs` protocol stores a complete-prefix marker
set in WSLDB and a strictly consecutive redundant acknowledgement stream on an
independently selectable oracle path. Unit tests prove that altered redundant
counters fail closed and that recovery accepts only equality or one complete
database transaction beyond the last complete oracle record.

Two fresh smoke cycles ran `init`, 50 finite `run` commits, and `verify`:

| Mode | Logical commits | Oracle commits | Entries | Transaction | Logical SHA-256 |
| --- | ---: | ---: | ---: | ---: | --- |
| Direct | 50 | 50 | 51 | 50 | `0ed94aa5cc6318ec9ad360520d2692a3b26f5e8615addef2727e68fa855ef889` |
| Buffered | 50 | 50 | 51 | 50 | `0ed94aa5cc6318ec9ad360520d2692a3b26f5e8615addef2727e68fa855ef889` |

Both oracle files had zero trailing bytes, both databases passed full
structural/value verification, and the two modes produced the same canonical
logical hash. Formatting, example unit tests, all-target all-feature check,
Clippy with warnings denied, release hygiene, and `git diff --check` passed.
`cargo package` then packaged 235 files (1.3 MiB uncompressed, 301.6 KiB
compressed) and compiled the unpacked crate successfully with the probe
included.

The actual APFS, ext4, and XFS evidence must follow
[Real power-loss testing](../power-loss-testing.md), place the oracle or ACK
collector outside the tested device's failure domain, record hardware and mount
assumptions, and retain failed as well as successful cycles.
