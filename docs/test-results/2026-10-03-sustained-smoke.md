# Sustained-update diagnostic and conformance — 2026-10-03

## Scope

This document records three different checks:

1. a symmetric warm-cache W5/W6 diagnostic used to find current bottlenecks;
2. an operation-limited run used to prove that the WSLDB and RocksDB adapters
   execute the same deterministic workload semantics;
3. the superseded asymmetric-cache diagnostic that exposed the harness defect.

None of these checks satisfies the preregistered 30-minute performance gate.

## Subjects and host

- Current timed-diagnostic WSLDB commit:
  `e9d691f89c085571a73aafe7c9c88246bc1574aa`
- Current semantic-conformance WSLDB commit:
  `b57d5c15ea2ee17d23be21c247fa73f3691db2cc`
- RocksDB commit: `abeebd9630f11bd08c28b7bd43c7bdfc62050654`
- Current RocksDB adapter SHA-256:
  `53e88b510c8212e79a4821c69fc93d40f76b46a3114d01745eb9917c9402994e`
- Host: Apple M1 Max, 64 GiB RAM, internal APFS SSD
- OS: macOS 26.6.2 (25G83), Darwin 25.6.0
- Rust: `rustc 1.89.0 (29483883e 2025-08-04)`

Raw commands, manifests, process output, resource measurements, and size tables
are retained outside the repository under:

- `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-current-head-native-sustained-smoke`
- `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-current-head-native-sustained-conformance`
- `/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-sustained-corrected-smoke`

## Symmetric warm-cache diagnostic

The original RocksDB adapter counted every entry with a public iterator before
the timer, which populated its block cache. WSLDB checked its metadata count and
did not touch its decoded-page cache. Commit `ab2285e` corrects this asymmetric
setup by traversing the complete logical dataset through both engines before
timing. WSLDB performs bounded 1,024-entry ranges rather than retaining the
complete dataset in memory. Both adapters report and validate the preload entry
count.

The clean diagnostic used 100,000 entries, 100-byte deterministic values, a
256 MiB cache setting, a 64 MiB WSLDB durable buffer, five seconds per process,
one warm-up, and three measured processes per engine and distribution. Engine
order alternated. Every mutation used WAL plus synchronous durability. Full
verification ran every two active seconds and final drain was measured
separately. Each table cell is the median of measured runs r1 through r3.

| W5 uniform metric | WSLDB | RocksDB | WSLDB relative result |
| --- | ---: | ---: | ---: |
| Active throughput | 756.167 ops/s | 831.391 ops/s | 9.0% lower |
| Point-read p50 | 4.292 us | 7.083 us | 39.4% lower |
| Point-read p99 | 45.708 us | 140.417 us | 67.4% lower |
| Overwrite p99 | 10.908 ms | 8.518 ms | 28.0% higher |
| Delete/recreate p99 | 17.930 ms | 18.446 ms | 2.8% lower |
| Final drain | 19.592 ms | 14.420 ms | 35.9% higher |
| Maximum RSS | 40,042,496 B | 24,199,168 B | 65.5% higher |
| Final logical storage | 12,338,522 B | 12,392,599 B | 0.44% lower |

| W6 Zipf 1.2 metric | WSLDB | RocksDB | WSLDB relative result |
| --- | ---: | ---: | ---: |
| Active throughput | 729.523 ops/s | 786.544 ops/s | 7.2% lower |
| Point-read p50 | 1.917 us | 2.958 us | 35.2% lower |
| Point-read p99 | 31.291 us | 126.083 us | 75.2% lower |
| Overwrite p99 | 11.221 ms | 10.569 ms | 6.2% higher |
| Delete/recreate p99 | 20.020 ms | 17.569 ms | 14.0% higher |
| Final drain | 11.281 ms | 18.030 ms | 37.4% lower |
| Maximum RSS | 39,649,280 B | 24,166,400 B | 64.1% higher |
| Final logical storage | 12,284,846 B | 12,328,915 B | 0.36% lower |

WSLDB wins warm point-read p50 and p99 in both distributions in this smoke
test. It does not show an overall W5 or W6 win: total throughput and RSS remain
worse, while durable write and final-drain tails are mixed. Five-second samples
are too short for a release claim, especially for sync latency and maintenance
behavior.

### Preload-memory diagnostic

The cache capacity is a limit, not an up-front allocation. Focused WSLDB-only
processes over the same 100,000-entry dataset measured 27,181,056 bytes maximum
RSS with caching disabled and 40,894,464 bytes with a 4,096-page limit; raising
the limit to 65,536 pages left RSS effectively unchanged at 40,910,848 bytes.
Both cached runs retained 2,962 pages. Reducing each public-API preload range
from 10,000 to 1,024 entries reduced the 65,536-page result to 39,960,576 bytes.

This removes avoidable benchmark high-water memory but does not explain away
the engine's resident cache. At one million entries, the current separate W3/W4
report shows WSLDB between 3.09% below and 1.87% above RocksDB maximum RSS, so
the large percentage here is primarily a small-dataset/fixed-overhead result.
Memory at larger sustained datasets remains an open release measurement.

### Durable-write latency attribution

The WSLDB operational counters separate complete commit time from time spent in
the required WAL sync. At the last complete maintenance sample in each current
measured process, sync accounted for 90.8% to 92.9% of W5 commit time and 91.7%
to 93.1% of W6 commit time. Average non-sync work was 375 to 483 microseconds
for W5 and 380 to 446 microseconds for W6; average sync time was 4.75 to 4.87
milliseconds for W5 and 4.94 to 5.15 milliseconds for W6.

The current durable-write bottleneck is therefore the one-sync-per-transaction
contract, not WAL encoding or tree lookup. Replacing the sync with a weaker
primitive would change durability and is not an admissible optimization.
Meaningful further gains require a separately specified group-commit path or
larger application transactions, followed by crash and power-loss validation.

## Superseded asymmetric-cache diagnostic

The following older results corrected WAL space accounting but started RocksDB
warm and WSLDB cold. They are retained to explain why the preload defect was
found and must not be used as the current comparative result.

The runner used a 100,000-entry database, 100-byte deterministic values, a
256 MiB cache setting, a 64 MiB WSLDB durable buffer, five seconds per process,
one warm-up, and three measured processes per engine and distribution. Engine
order alternated. Every mutation used WAL plus synchronous durability. Full
verification ran every two active seconds and final drain was measured
separately.

The size calculation includes both the WSLDB main file and its WAL. An earlier
diagnostic omitted the WAL and is not used here. Each table cell below is the
median of measured runs r1 through r3.

| W5 uniform metric | WSLDB | RocksDB | WSLDB relative result |
| --- | ---: | ---: | ---: |
| Active throughput | 776.422 ops/s | 865.802 ops/s | 10.3% lower |
| Point-read p50 | 5.625 us | 5.916 us | 4.9% lower |
| Point-read p99 | 208.042 us | 78.208 us | 2.66x higher |
| Overwrite p99 | 9.442 ms | 8.157 ms | 15.8% higher |
| Delete/recreate p99 | 17.951 ms | 13.276 ms | 35.2% higher |
| Final drain | 16.218 ms | 17.122 ms | 5.3% lower |
| Maximum RSS | 40,206,336 B | 23,412,736 B | 71.7% higher |
| Final logical storage | 12,339,404 B | 12,396,704 B | 0.46% lower |

| W6 Zipf 1.2 metric | WSLDB | RocksDB | WSLDB relative result |
| --- | ---: | ---: | ---: |
| Active throughput | 782.805 ops/s | 867.980 ops/s | 9.8% lower |
| Point-read p50 | 1.083 us | 2.209 us | 51.0% lower |
| Point-read p99 | 114.250 us | 40.667 us | 2.81x higher |
| Overwrite p99 | 10.433 ms | 8.227 ms | 26.8% higher |
| Delete/recreate p99 | 17.692 ms | 14.386 ms | 23.0% higher |
| Final drain | 18.582 ms | 15.646 ms | 18.8% higher |
| Maximum RSS | 39,731,200 B | 23,379,968 B | 69.9% higher |
| Final logical storage | 12,286,106 B | 12,331,655 B | 0.37% lower |

The apparent read-tail loss in these tables was primarily a cache-state setup
error. Their write, storage, and RSS observations remain diagnostic only; the
later clean symmetric run above is the current short comparison.

### Rejected metadata-only commit probe

Commit `2b3e097` tested replacing the full checkpoint-root validation performed
before every buffered commit with a checksummed metadata-only probe. The clean
rerun is retained under
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-03-sustained-metadata-probe-smoke`.
The host was slower during that run, so RocksDB was used as a simultaneous
control: the median WSLDB/RocksDB throughput ratio worsened by 2.8% on W5 and
3.5% on W6, while write and read tails were mixed and noisy. The change did not
produce a defensible win and also removed an extra root-integrity check from the
commit path. It was reverted in `c9dbc3e`; full root validation remains.

### Rejected Linux `sync_data` WAL probe

Commit `5681a26` was measured on the same virtualized Linux/ext4 environment as
the Linux correctness gates. A one-line candidate replaced `sync_all` with
`sync_data` for append-only WAL publication. Both variants used five fresh
copies of the same 100,000-entry database, a 64 MiB durable buffer, a complete
warm preload, and an exact 5,000-operation W5 uniform prefix. Every run reached
the limit with 100,000 entries and the same logical SHA-256.

The `sync_all` baseline median was 12,923.783 ops/s; the `sync_data` candidate
median was 12,807.804 ops/s, 0.897% lower. The candidate therefore showed no
measured benefit and was discarded. WSLDB retains `sync_all`; reducing the
number of durable synchronization points requires an explicit group-commit
contract rather than substituting a narrower primitive.

Raw local evidence is retained under
`wsldb-benchmark-evidence/2026-10-03-linux-ext4-wal-sync-baseline-5681a26`
and
`wsldb-benchmark-evidence/2026-10-03-linux-ext4-wal-sync-data-candidate`.

## Operation-limited semantic conformance

Timed processes finish different operation counts, so their final logical
states must not be compared directly. The adapters now accept an optional exact
operation limit. The runner fails unless both reach that limit with the same
entry count and logical SHA-256.

The current clean conformance run on `b57d5c1` used 10,000 initial entries, a
complete preload in both engines, 1,000 deterministic operations, both uniform
and Zipf 1.2 distributions, and two independent runs per distribution. Every
comparison passed:

| Distribution | WSLDB operations | RocksDB operations | Entries | Identical logical SHA-256 |
| --- | ---: | ---: | ---: | --- |
| Uniform, run 0 | 1,000 | 1,000 | 10,000 | `ff61aa66...9fa7` |
| Uniform, run 1 | 1,000 | 1,000 | 10,000 | `ff61aa66...9fa7` |
| Zipf 1.2, run 0 | 1,000 | 1,000 | 10,000 | `d5ee4be0...1c2d` |
| Zipf 1.2, run 1 | 1,000 | 1,000 | 10,000 | `d5ee4be0...1c2d` |

This proves the two sustained adapters generate equivalent logical results for
the checked deterministic prefixes. It does not by itself prove database
durability under power loss or comparative long-duration performance.
