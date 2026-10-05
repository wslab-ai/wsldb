# WSLDB file format

Status: unreleased development format, documented for compatibility testing but
not yet promised stable across releases. All integers are unsigned
little-endian. Checksums are CRC32C (Castagnoli), with the standard check value
`0xe3069283` for `123456789`. Page IDs are zero-based physical page indexes.

## File layout

The page size is fixed at 4096 bytes.

| Page IDs | Purpose |
| --- | --- |
| 0–1 | Alternating metadata slots |
| 2 and above | Immutable leaf, branch, or overflow pages |

A normal commit appends immutable data pages, syncs them, and then publishes the
new root in metadata slot `transaction_id % 2`. Readers choose the valid slot
with the highest transaction ID. A child or overflow continuation must always
have a smaller page ID than its parent.

A newly initialized, backed-up, or compacted file may contain only one valid
metadata head. That head is still written to `transaction_id % 2`, leaving the
opposite slot invalid. The next transaction therefore publishes into the other
slot and cannot destroy the only recovery head with a partial metadata write.
A checksummed head stored in the opposite slot is noncanonical and invalid.

Normal opening defines slot validity from the metadata and root page and checks
deeper pages lazily. Full verification on open may reject a deeply corrupt
newest tree and select the newest older fully verified slot only when no WAL
existed before recovery. It does not rewrite either slot. A nonempty WAL
forbids this fallback because it may contain durable transactions newer than
the older page-tree snapshot.

## Metadata page

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 8 | Magic `WSLDB\0\0\0` |
| 8 | 4 | Reserved, written as zero |
| 12 | 4 | Page size, `4096` |
| 16 | 8 | Transaction ID |
| 24 | 8 | Root page ID, or zero for an empty tree |
| 32 | 8 | Highest page ID belonging to this commit boundary |
| 40 | 8 | Logical entry count |
| 48 | 4 | CRC32C of bytes 0–47 |
| 52 | 4044 | Reserved, written as zero |

An empty database has transaction ID zero, no root, last page ID one, and zero
entries. A non-empty root must be in the data-page range and no greater than the
last page ID. Root absence and a zero entry count must agree.

## Common data-page header

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | Magic `WSLP` |
| 4 | 1 | Reserved, written as zero |
| 5 | 1 | Kind: leaf `1`, branch `2`, overflow `3` |
| 6 | 2 | Reserved, written as zero |
| 8 | 4 | Payload length |
| 12 | 8 | Creating transaction ID |
| 20 | 4 | CRC32C of the used payload bytes |
| 24 | 4 | CRC32C of header bytes 0–23 followed by the expected `u64` page ID |
| 28 | 4 | Reserved, written as zero |
| 32 | 4064 | Payload followed by zero padding |

The creating transaction ID must be non-zero and no newer than the snapshot
that references the page. Binding the expected physical page ID into the header
checksum makes copying a valid page into a different slot detectable. Payload
length cannot exceed 4064 bytes.

## Leaf payload

The payload begins with a `u16` entry count, a `u16` common-key-prefix length,
and the common prefix bytes stored once for the page. Each entry is:

1. `u16` key-suffix length;
2. key-suffix bytes;
3. `u8` value representation kind;
4. representation-specific fields.

Inline value kind `1` stores a `u32` byte length followed by the raw value.
Overflow value kind `2` stores an `u64` overflow-head page ID, `u64` total value
length, and `u32` CRC32C of the complete reconstructed value.

The decoded key is the page prefix followed by its entry suffix. Decoded keys
are non-empty, at most 1024 bytes, and strictly increasing bytewise within the
leaf. Inline values are at most 2048 bytes. Larger values, up to 64 MiB, use
overflow pages.

## Branch payload

The payload begins with a `u16` child count and the first `u64` child page ID.
Every remaining child is encoded as `u16` separator length, separator bytes, and
`u64` child page ID. A branch has at least two children, exactly one fewer
separator than children, and strictly increasing non-empty separators.

Separator `i` is exactly the minimum key of child `i + 1`. Child ranges must not
overlap, and all leaves below one root have the same depth.

## Overflow payload

The first eight payload bytes are the next overflow page ID, or zero at the end
of the chain. Remaining bytes are the value chunk and must be non-empty. A leaf
records the expected total length and whole-value checksum, so a chain that is
short, long, reordered, duplicated, or altered fails validation.

## Compatibility and migration

This is an unversioned, unreleased format with a pre-release compatibility
baseline. The first stable release will freeze its encoded bytes. Reserved
fields must be zero and are not assigned any meaning yet.

The encoder tests contain golden byte vectors for the complete used portion of
an empty metadata header and a two-entry inline leaf. Changing either vector
requires an intentional compatibility decision.

Committed decoder fixtures under `tests/fixtures` extend that baseline:

- `database.wsldb` contains two direct commits, a multi-level tree, inline and
  overflow values, replacements, and deletions;
- `buffered.wsldb` plus `buffered.wsldb.wal` contain a page-tree checkpoint, a
  consolidated snapshot record, and two later transaction records.

The fixture test copies these bytes before opening them, validates the complete
logical contents, and runs full structural verification. The release audit
also rejects any byte change unless its full SHA-256 allowlist is changed in an
explicit compatibility commit. Their SHA-256 values are respectively
`eb64a848...3d9e52`, `c030112b...53cfc2`, and `e5967eb6...8a64a`. They have
deliberately unversioned names because no format version has been released.
Regeneration is an explicit compatibility action:

```text
WSLDB_WRITE_FORMAT_FIXTURES=tests/fixtures \
  cargo test --test format_fixtures -- --exact \
  committed_format_fixtures_remain_readable
```

After the first stable release, an incompatible change will additionally
require a tested migration path; silently regenerating fixtures is not an
acceptable substitute.

## Optional write-buffer sidecar

When durable write buffering is enabled, `<database>.wal` contains consecutive
physical records. These may be individual transactions, a compact transaction
group, or one consolidated snapshot followed by later records. There is no
file-level header. An individual transaction or snapshot record contains:

| Field | Size |
| --- | ---: |
| Magic `WSLDBWAL` for a transaction or `WSLDBSNP` for a snapshot | 8 bytes |
| Complete record length | 8 bytes |
| Base transaction ID | 8 bytes |
| New transaction ID | 8 bytes |
| Resulting logical entry count | 8 bytes |
| Mutation count | 4 bytes |
| Payload CRC32C | 4 bytes |
| Header CRC32C | 4 bytes |
| Sorted mutation payload | variable |
| Record CRC32C | 4 bytes |
| Footer magic `WEND` | 4 bytes |
| Repeated record length | 8 bytes |

An individual-record mutation is a `u16` common-prefix length relative to the
preceding key, a `u16` key-suffix length, the suffix bytes, and a `u64` value
length followed by value bytes. The first prefix length is zero. Length
`u64::MAX` is a tombstone. Reconstructed keys must be non-empty, at most 1024
bytes, and strictly ordered within a logical transaction. Header, payload, and
complete-record checksums are independent. The footer makes an incomplete
physical tail distinguishable from a complete corrupt record.

Recovery validates the physical length against the 1 GiB record limit before
reading the body and reserves the exact record buffer fallibly. Capacity
failure returns `Error::FileTooLarge` without decoding or accepting any part of
the record. Overflow-value reconstruction and bounded maintenance staging use
the same fallible-allocation rule.

A compact group uses magic `WSLDBGRP` and the same 52-byte header and 16-byte
footer. Its base transaction ID is the first logical transaction's base, its
new transaction ID and entry count describe the final logical transaction, and
the header's count field is the number of logical transactions. Their IDs are
the consecutive successors of the base. The group payload stores, for each
logical transaction in order:

1. resulting entry count as a canonical unsigned varint;
2. mutation count as a canonical unsigned varint;
3. the sorted mutations.

A group mutation stores canonical varints for common-prefix length, key-suffix
length, and tagged value length. Tag zero is a tombstone; any other tag is the
value length plus one, so an empty value has tag one. Key-prefix compression
continues across logical transaction boundaries, while strict key ordering is
required separately within each logical transaction. The decoder verifies the
whole physical record, every length and count, final transaction and entry
summary, and every logical mutation before yielding any transaction.

Physical records form one continuous logical transaction chain: after the
first retained record, every base must equal the preceding transaction. The
retained chain may begin before the selected page-tree checkpoint because
checkpoint publication makes metadata durable before truncating the WAL.
Recovery skips that covered prefix and requires either a record ending exactly
at the checkpoint with the same entry count, or a first newer record whose base
is exactly the checkpoint. Only records after that boundary are replayed. A WAL
whose last transaction is not newer than the selected checkpoint is stale
checkpoint residue and is cleared.

A transaction record advances its encoded base transaction by exactly one. A
snapshot record may appear only first, advances from its encoded base to any
newer transaction, and contains the latest mutation for every key changed in
that interval. When replayed beyond the page-tree checkpoint its base must be
that checkpoint; a retained snapshot ending exactly at the checkpoint is an
already covered boundary. Explicit flush writes and syncs the complete snapshot
to a temporary sidecar before atomically replacing the WAL on Unix. Either file
is a complete representation of the same logical head. Later transaction
records again advance one transaction at a time.

An explicit or automatic group commit encodes its batches in one compact group
record of at most 1 GiB, performs one positioned write, and performs one WAL
sync. In-memory visibility advances to the final logical transaction only after
that sync succeeds. A partial write is truncated back to the previous durable
boundary and the truncation is synced before a definite error is returned.
Files written before compact grouping remain readable because their
concatenated individual transaction records retain distinct magic.

If a process or machine stops before the group call returns, recovery may find
the old head or the complete compact group; a torn group never yields a logical
prefix. This remains an unknown-outcome rule: applications compare the
recovered transaction ID before retrying. After a successful return, every
logical transaction through the returned ID has passed the shared sync.
