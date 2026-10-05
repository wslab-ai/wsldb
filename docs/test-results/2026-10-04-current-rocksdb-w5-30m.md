# Current 30-minute W5 RocksDB diagnostic

Date: 2026-10-04.

## Subject

- WSLDB commit: `04fb045d404b1a12ceb2d112066ff3814b205a4b`.
- RocksDB: clean official `v11.8.1` commit
  `abeebd9630f11bd08c28b7bd43c7bdfc62050654`.
- RocksDB adapter SHA-256:
  `492fc9798912fcde5ea4ad26df9c30fffba67c89a3d6e3ec854f7e6c1079d331`.
- Host: Apple M1 Max, macOS 26.6.2 (25G83), APFS on the internal SSD.
- Both runs used 1,000,000 keys, incompressible 100-byte values, a 256 MiB
  cache limit, the frozen uniform W5 operation stream, WAL, and synchronous
  durability for every mutation.

The WSLDB result is the current single-process 30-minute run documented
[separately](2026-10-04-current-w5-30m.md). The RocksDB adapter first loaded
the byte-identical initial dataset and reproduced its logical SHA-256. Its
timed process then performed the same warm preload, 1,800-second deadline,
per-second latency windows, periodic full logical hashes, final flush and
compaction, checksum verification, and independent reopen.

## Observed result

| Metric | WSLDB | RocksDB 11.8.1 | WSLDB relative result |
| --- | ---: | ---: | ---: |
| Active operations | 1,195,318 | 1,177,583 | 1.51% more |
| Active throughput | 674.633 ops/s | 661.587 ops/s | 1.97% higher |
| Wall-window throughput | 664.064 ops/s | 654.213 ops/s | 1.51% higher |
| Point-read p99 | 105.000 us | 229.041 us | 54.16% lower |
| Overwrite p99 | 13.800 ms | 10.634 ms | 29.77% higher |
| Delete/recreate p99 | 23.245 ms | 18.754 ms | 23.95% higher |
| Verification median | 976.395 ms | 838.299 ms | 16.47% higher |
| Final drain | 191.088 ms | 3,404.849 ms | 94.39% lower |
| Maximum RSS | 240,943,104 B | 313,212,928 B | 23.07% lower |
| Final logical storage | 148,316,248 B | 122,025,182 B | 21.55% higher |

Both engines preserved exactly 1,000,000 entries and passed their final full
verification and independent reopen. WSLDB's final storage is its main file
plus consolidated WAL. RocksDB's final storage includes all files after
`WaitForCompact` flushed every column family and waited for already scheduled
background compaction and purge work. The adapter does not request a manual
full-range compaction.

## Long stall

The aggregate throughput result is dominated by one RocksDB window. Window
1471 contained 218 operations and lasted 279.068 seconds because one durable
delete/recreate cycle took 278.778 seconds. The immediately preceding and
following windows returned to 836.607 and 696.373 operations/s. Excluding only
that window gives RocksDB 784.456 operations/s, 16.28% above WSLDB's complete
run. The stall is retained because W5 explicitly requires reporting latency
over time so stalls cannot be averaged away.

This single sequential diagnostic cannot identify whether the stall originated
inside RocksDB, APFS, the SSD, or unrelated host activity. It therefore does
not prove a general WSLDB throughput win and must not replace the frozen
alternating repeated comparison. It does prove that the short five-second
smoke result is insufficient to characterize long-tail behavior.

Raw setup, run, resource, file-size, and independent-reopen evidence is
retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-rocksdb-w5-30m`.
The main RocksDB log SHA-256 is
`4066e6568663c106c4d8e5a19cc70c4acaf059b28d92a6b59d8b6269462b2b02`.
