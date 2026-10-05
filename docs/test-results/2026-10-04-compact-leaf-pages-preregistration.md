# Compact leaf-page encoding preregistration

Date: 2026-10-04. This document freezes the hypothesis, implementation scope,
workload, and acceptance gates before changing the page encoder.

## Hypothesis and prior art

WSLDB's current leaf pages already store one common key prefix per page, which
follows the prefix B-tree result of Bayer and Unterauer. The remaining entry
metadata uses a fixed two-byte key-suffix length and a fixed four-byte inline
value length. LevelDB's immutable table blocks demonstrate the narrower idea
being tested here: canonical variable-length integers for shared-key,
unshared-key, and value lengths, combined with prefix compression.

This candidate applies only that representation technique to WSLDB's existing
copy-on-write B+ tree. It does not import LevelDB/RocksDB code, introduce an
LSM tree, change transaction semantics, or weaken durability.

Primary references:

- Rudolf Bayer and Karl Unterauer, “Prefix B-Trees”, ACM TODS 2(1), 1977:
  <https://doi.org/10.1145/320521.320530>
- LevelDB block builder, including prefix lengths and restart structure:
  <https://github.com/google/leveldb/blob/main/table/block_builder.cc>
- LevelDB block decoder, including bounded varint decoding:
  <https://github.com/google/leveldb/blob/main/table/block.cc>

For the frozen 16-byte-key/100-byte-value workload, a canonical varint should
normally replace two suffix-length bytes with one byte and four value-length
bytes with one byte. The expected saving is therefore about four bytes per
inline leaf entry, before secondary savings from needing fewer leaf and branch
pages. This directly targets the remaining W6 storage debt without touching
the fsync-dominated single-caller commit path.

## Frozen implementation boundary

1. Add a distinct compact-leaf page kind. Newly written leaf pages use it.
2. Retain the existing leaf decoder unchanged for old databases and the
   committed compatibility fixture. This is a self-identifying record variant,
   not a numbered whole-file format.
3. Encode suffix and inline-value lengths as canonical unsigned varints.
4. Keep the page size, page header, page and payload checksums, page-wide common
   prefix, value bytes, overflow descriptor, copy-on-write publication order,
   and all public APIs unchanged.
5. Share one canonical varint implementation between WAL groups and pages.
6. Reject truncated, overflowing, and non-canonical varints before exposing a
   decoded page.

Branch-page compression, restart-point search inside encoded bytes, changes to
overflow storage, compression of user values, and durability changes are out
of scope. They require separate preregistrations.

## Frozen causal workload

The independent baseline is commit
`5b5b953b33f8d5d0a8d22ac00fd8ad3da04757c1`; its release benchmark SHA-256 is
`d4c1a0ff781bbdb1f18e7baca73b7d492bd44c9f20e1fe1f82528cf4f1bd1a45`.

To keep the laptop cool and fit the available disk, the causal diagnostic uses
100,000 ascending 16-byte keys, deterministic incompressible 100-byte values,
10,000 mutations per durable commit, an 8,192-page cache, and fresh database
files. One unreported warm-up precedes five alternating baseline/candidate
pairs. Each retained database is checked for exactly 100,000 entries and the
same logical SHA-256. The same binaries then run W3/t16 with 100,000 operations
and W4/range-100 with 10,000 operations against their own files, one warm-up
and five measured samples as implemented by the frozen benchmark harness.

This short diagnostic decides whether the codec is worth retaining. It is not
a replacement for the preregistered one-million-entry workloads or W6.

## Acceptance gates

The candidate is retained only if all of the following hold:

- every baseline and candidate database has an identical entry count and
  logical SHA-256;
- the old committed format fixture and an explicitly constructed legacy leaf
  page remain readable;
- new compact pages round-trip empty, 100-, 127-, 128-, and 2,048-byte inline
  values plus overflow descriptors;
- focused tests reject truncated, overflowing, and non-canonical varints, and
  page fit/split tests cover the exact payload boundary;
- median final logical storage is at least 2% smaller than baseline;
- median W1 throughput, W3/t16 throughput, W4/range-100 throughput, full
  verification time, and maximum RSS do not regress by more than 3%;
- formatting, the complete all-feature test suite, all-target check, Clippy
  with warnings denied, and `git diff --check` pass.

If any gate fails, the candidate is reverted or retained only as documented
negative evidence. A new RocksDB superiority claim requires a later fresh,
pinned, alternating comparison; this experiment alone cannot establish it.

## Result: rejected

The implementation preserved the logical SHA-256
`b2cafaff4e09147b6fdbd0e8fddfe8931e18ae0a5c1718dff8b4b9d4a3e0533d`
and exactly 100,000 entries in every run. Candidate benchmark SHA-256 was
`bc93b391d24a42d7032c08869f7c12ea3f36d6dedeeec7efd4e739fc505e88ec`.

| Metric | Baseline median | Candidate median | Change |
| --- | ---: | ---: | ---: |
| W1 throughput | 421,527.849 ops/s | 413,509.920 ops/s | -1.90% |
| W3/t16 throughput | 18,249,284.126 ops/s | 15,126,972.784 ops/s | -17.11% |
| W4/range-100 throughput | 143,814.278 ranges/s | 136,805.456 ranges/s | -4.87% |
| Final logical storage | 12,251,136 B | 11,620,352 B | -5.15% |

The compact representation cleared the storage and W1 gates but failed the
frozen W3 and W4 gates. The 100,000-operation W3 interval is short enough for
thread scheduling to contribute substantial variance, but that observation
does not permit changing the preregistered decision rule after seeing the
result. The codec is therefore rejected and its source changes are reverted.
No RocksDB performance claim follows from this experiment.

Raw binaries, commands, stdout, stderr, run order, file sizes, resource
measurements, and logical hashes are retained at
`/Users/eugene/Projects/wsldb-benchmark-evidence/2026-10-04-compact-leaf-pages`.
