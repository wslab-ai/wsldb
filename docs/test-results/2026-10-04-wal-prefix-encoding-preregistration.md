# Prefix-compressed WAL keys

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation

The accepted one-million-entry W2 database occupies 123,446,916 logical bytes,
1.23% more than RocksDB's 121,951,653 bytes. The consolidated WAL stores every
sorted 16-byte key in full even though adjacent keys share a long prefix. The
page format already exploits the same ordering property within leaves.

## Candidate

Every mutation in a WAL record will encode the common-prefix length with the
preceding key, the remaining suffix length and bytes, and the existing value
length/value. Both lengths remain `u16`; the first key must have a zero prefix.
The decoder reconstructs keys incrementally and retains strict-order,
maximum-length, payload, header, and record checksum validation. Streaming
snapshot publication computes the same bytes and CRC32C without materializing
the record.

WSLDB has no released format and the documented development format is
explicitly unversioned, so this replaces the pre-release WAL encoding rather
than adding an internal format version. The committed buffered decoder fixture
and its documented hash must be regenerated intentionally. Main-file bytes do
not change.

## Frozen acceptance gate

The causal A/B uses one million unique 16-byte keys, incompressible 100-byte
values, 10,000 mutations per synchronous commit, a 64 MiB durable write
buffer, and 65,536 decoded cache pages. The baseline is storage commit
`f2a792c`; one warm-up and five measured fresh-file processes per variant are
alternated, and every process explicitly drains pending work.

The candidate is accepted only if final WSLDB logical storage becomes smaller
than the fresh RocksDB reference of 121,951,653 bytes, median active throughput
does not regress by more than 2%, median active-plus-drain time does not regress
by more than 3%, and median maximum RSS does not grow by more than 5%. Median
maximum commit latency may not exceed 25 milliseconds. Logical hashes must
match the baseline, output must be deterministic within each variant, and
checkpoint failures must remain zero.

Focused tests must prove round trips and exact streaming/buffered byte equality
for zero, partial, full, and invalid prefixes; repeated-key updates,
tombstones, maximum-width keys, truncation, checksum corruption, refresh,
group commit, crash recovery, and the regenerated committed fixture must pass.
Formatting, all-target all-feature tests, Clippy with warnings denied, check,
diff-check, and the 700-line source limit must pass. Thresholds will not change
after candidate results are observed.

## Result: accepted

The candidate at `4d3e59c` passed every frozen condition. One warm-up per
variant preceded five alternating measured fresh-file processes. The baseline
was the accepted `f2a792c` storage binary.

| Median metric | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Logical storage | 123,446,916 B | 121,155,676 B | -1.86% |
| Active W2 throughput | 596,283.040 ops/s | 606,457.836 ops/s | +1.71% |
| Active load time | 1.677 s | 1.649 s | -1.68% |
| Active plus drain time | 1.912 s | 1.873 s | -2.02% |
| Standalone drain | 219.747 ms | 219.755 ms | +0.00% |
| Maximum commit latency | 20.546 ms | 19.485 ms | -5.16% |
| Maximum sync latency | 34.297 ms | 33.119 ms | -3.44% |
| Maximum RSS | 203,358,208 B | 205,225,984 B | +0.92% |

Candidate storage is 795,977 bytes, or 0.65%, smaller than
the fresh RocksDB reference of 121,951,653 bytes. The candidate main file grew
from 65,486,848 to 67,911,680 bytes because compressed WAL accounting crosses
the checkpoint threshold later; the final WAL fell from 57,960,068 to
53,243,996 bytes. The combined result is smaller, and all five candidate runs
produced exactly those lengths.

Every process reported zero checkpoint failures and logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
Baseline main/WAL hashes stayed
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`
and
`21ba0081c157f7ca7a34adc93fd26e72c8ea3623abff1261d0be43cc9081eedd`;
candidate main/WAL hashes were deterministically
`eb31eac2b99fdbaf9107be51343850b5aeaec2ee4d5e333959dbc926e1cb43ea`
and
`ddc8a386482bbb01815427004bcd70932be35d59933ee3efc91164f773641788`.

The committed buffered decoder fixture was regenerated intentionally. Its WAL
SHA-256 is now
`e5967eb6e30ff9ab4b06a830f56a96eb08672f70d1fee71ae6a2a9312af8a64a`;
the main-file fixtures did not change. Focused and complete all-feature tests
cover prefix boundaries, maximum-width keys, invalid prefixes, tombstones,
streaming byte equality, fixture reopen, refresh, group commit, short writes,
fault injection, corruption, and every existing crash boundary.

The baseline and candidate benchmark SHA-256 values are respectively
`49fbecf164e71f3fd37bda11c6698d48c00e6ad4ad76ed03e44959398c03d7dc`
and
`584140f31ebd949eab3a0ac28673cabcdd04f3bfa080eee41c6d69d7cac34ed1`.
Raw outputs, resource reports, hashes, sizes, binaries, and retained databases
are outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-wal-prefix-w2-ab`.
