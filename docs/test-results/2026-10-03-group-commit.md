# Explicit group-commit evidence — 2026-10-03

## Subject and contract

- WSLDB commit: `df81a6927d5a4b2c5009a40beda7999d7dcec9bd`
- Image: official `rust:1.89-alpine`
- Kernel: Linux 6.8.0-100-generic, arm64, inside a Colima virtual machine
- Filesystem: ext4 guest root
- Logical transactions per mode: 4,096
- Mutations per transaction: one 16-byte key and one deterministic 100-byte value
- Explicit group size: 16 transactions
- Measured processes: five, alternating mode order
- Durable buffer threshold: 1 GiB, so no measured checkpoint occurs

Both modes use the same buffered WAL record format and synchronous durability.
The sequential mode calls `WriteTransaction::commit` once per logical
transaction. The grouped mode prepares the same mutations as individual
`WriteBatch` values and submits 16 records to `Database::commit_group`, which
uses one positioned write and one sync.

## Results

| Run | Sequential tx/s | Grouped tx/s | Sequential syncs | Grouped syncs |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 2,931.371 | 24,263.769 | 4,096 | 256 |
| 2 | 3,005.280 | 25,576.589 | 4,096 | 256 |
| 3 | 3,093.552 | 28,606.554 | 4,096 | 256 |
| 4 | 3,004.559 | 28,604.197 | 4,096 | 256 |
| 5 | 2,990.882 | 26,280.955 | 4,096 | 256 |
| Median | 3,004.559 | 26,280.955 | 4,096 | 256 |

The median grouped throughput is 8.747x the sequential baseline. The number of
syncs falls exactly 16x. Every run produced 794,624 WAL bytes in both modes and
the same logical SHA-256:
`cbd8ba5aeeb849af1dc5ba4c266622880cf776788942a74e7044c16a73d4b65d`.
The benchmark left no database or sidecar path behind.

This is a local WSLDB A/B measurement of an explicit application-selected
group size. It is not a WSLDB-versus-RocksDB result, does not predict other
filesystems or devices, and does not alter the frozen W5/W6 comparison.

Raw local evidence:
`wsldb-benchmark-evidence/2026-10-03-linux-ext4-group-commit-df81a69`.

## Correctness coverage

The same commit passed the complete 122-test suite on macOS and Linux/ext4.
Focused tests cover ordered and repeated keys, empty batches, immutable old
readers, reopen, automatic checkpoint, stale handles, a complete-record prefix
followed by a partial write, sync outcome ambiguity, process crash after the
shared sync, and corruption/truncation recovery.

The finite Linux corruption gate additionally mutated 100,000 main-file cases
and 100,000 WAL cases. Its WAL baseline contains the two records issued by one
group call and accepts only the old checkpoint, the complete first-record
prefix, or the complete group as valid logical states.
