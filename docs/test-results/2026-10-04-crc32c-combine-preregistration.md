# Combined WAL checksum

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

The post-batching W2 profile identifies WAL encoding as the largest
controllable CPU symbol. Every record currently scans its payload once for the
payload CRC32C and then scans the same bytes again as part of the complete
header-plus-payload CRC32C.

The candidate will add a GF(2) CRC32C combine operation. Given the finalized
CRC of the complete header, the finalized payload CRC, and the payload length,
it derives the CRC of their concatenation in logarithmic time without reading
the payload again. Header, payload, footer, checksum values, byte order, and all
persistent-format fields remain unchanged.

## Frozen acceptance gate

The causal A/B uses one-million 16-byte keys, 100-byte values, 10,000 mutations
per synchronous commit, a 64 MiB durable write buffer, and 65,536 decoded cache
pages. It runs one warm-up and five measured fresh-file processes per variant,
alternating measured variants.

The candidate is accepted only if median active W2 throughput improves by at
least 3%, median maximum RSS grows by no more than 2%, every main-file, WAL, and
logical hash matches, exhaustive deterministic split tests prove
`combine(crc(a), crc(b), len(b)) == crc(a || b)`, WAL corruption tests pass, and
the complete formatting, test, Clippy, check, and diff-check gate passes.
Thresholds will not be changed after results are observed.

## Result

Accepted. The preregistered one-warm-up, five-measured-run alternating A/B was
run from fresh files. Median active W2 throughput increased from 421,054.220 to
439,456.539 operations/s, a 4.37% improvement. Median maximum commit latency
decreased from 353,902,916 to 344,876,667 nanoseconds. Median maximum RSS moved
from 177,258,496 to 177,324,032 bytes, a 0.037% increase. The recorded
`sync_total_ns` metric, which includes WAL encoding and append work around the
sync, decreased from 945,106,711 to 867,579,496 nanoseconds.

All ten measured runs produced the same logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`, main-file
hash `f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948`, and WAL
hash `112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68`.
Main-file and WAL lengths remained 65,486,848 and 57,963,128 bytes. Raw output
is retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-crc32c-combine-w2-ab`.

The deterministic test covers every split of an 8,193-byte input. The complete
all-target, all-feature test suite, Clippy with warnings denied, `cargo check`,
format check, and diff check passed. The persistent format and durability
protocol are unchanged.
