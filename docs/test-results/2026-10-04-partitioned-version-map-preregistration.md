# Partitioned version-map application

Date: 2026-10-04. Host: Apple M1 Max, 64 GiB RAM, APFS, macOS 26.6.2.
This acceptance contract is recorded before implementation or measurement.

## Observation and candidate

The post-CRC W2 profile attributes 73 flat samples to applying a durable WAL
record to the ordered version map. The point-index preflight already proves,
for every mutation in an individual commit, whether its key is present in that
map. Nevertheless, the current publication path performs a full growing-map
entry search for every new key.

The candidate will consume the validated presence vector after the WAL sync.
Existing keys keep the current single-entry traversal so their historical
version chain is unchanged. New keys are accumulated in their own ordered map
and appended to the existing disjoint map as one ordered merge. Explicit group
commit and recovery replay retain the general per-key path because later
records can depend on earlier records in the same group or replay sequence.

Point-index validation, WAL bytes, sync order, reader publication, transaction
IDs, and persistent-format bytes remain unchanged. The specialized path is
eligible only after the existing preflight has validated the complete presence
vector before durability begins.

## Frozen acceptance gate

The causal A/B uses one-million unique 16-byte keys, 100-byte values, 10,000
mutations per synchronous commit, a 64 MiB durable write buffer, and 65,536
decoded cache pages. It runs one warm-up and five measured fresh-file processes
per variant, alternating measured variants.

The candidate is accepted only if median active W2 throughput improves by at
least 3%, median maximum RSS grows by no more than 2%, every main-file, WAL, and
logical hash matches, focused mixed new/update and historical-version tests
pass, and the complete formatting, all-target all-feature test, Clippy, check,
and diff-check gate passes. Maximum commit and sync latency are reported.
Thresholds will not be changed after results are observed.

## Result

Rejected. The run had substantial filesystem-sync variance, but the frozen
median remains authoritative: baseline active W2 throughput was 364,198.626
operations/s and the candidate reached 360,278.519 operations/s, a 1.08%
regression. Median maximum commit latency decreased from 558,950,250 to
403,707,583 nanoseconds, median recorded `sync_total_ns` decreased from
1,047,156,958 to 1,020,021,211 nanoseconds, and median maximum RSS decreased
from 177,029,120 to 173,359,104 bytes. These secondary improvements do not
override the throughput gate.

Every warm-up and measured run produced logical hash
`1650dd46f185ebcddbb067b55ca51dea49bc39d3fa8de561bcdd255f37a7410e`.
All measured main files had hash
`f6a4b6f8d1854f32e91e80fd2698d39dc0c922481b20579eb3543fc1b08a1948` and
length 65,486,848 bytes; every WAL had hash
`112f6668210009b69372e71e9f70bdd2eba61379060b59e030260972c0b03e68` and
length 57,963,128 bytes. Raw evidence is retained outside the repository at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-partitioned-version-map-w2-ab`.

The specialized merge and its test were removed. The general per-key
application path remains unchanged; only its source-file split is retained.
