# Public API stability

No WSLDB release has been published. Until the first release, the public API
may still change when correctness or operability requires it. The release
candidate must pass the external-consumer contract in `tests/public_api.rs`.
The local release audit pins that contract's exact SHA-256 value, so changing a
promised type, method, field, error variant, or auto trait requires an explicit
contract review and checksum update. The contract test is included in the Cargo
package and compiles as an external crate.

This checksum is change control for the pre-release contract definition, not a
claim that the API is already stable. The first approved release artifact is
the semantic-versioning baseline. A newly exported item must also be added to
the compile contract before that release; the checksum alone cannot discover
an export omitted from the test.

Resource limits that callers need for admission control are part of that
surface. `MAX_KEY_BYTES` and `MAX_VALUE_BYTES` expose the accepted mutation
sizes, and `MAX_GROUP_TRANSACTIONS` defines the maximum explicit group size;
automatic grouping transparently splits a larger pending queue. The
pre-release contract also includes
`DatabaseOptions::verify_on_open(bool)`, allowing callers to choose explicitly
between bounded fast-open validation and a complete pre-return logical scan.
`ReadTransaction::range_after` is the bounded pagination contract: `after` is
exclusive, `end` is exclusive, and successive calls on one immutable read
transaction cannot repeat a boundary key.

After the first release, WSLDB follows semantic versioning for every item
exported from the crate root. Removing an item, narrowing a method signature,
changing transaction semantics, or removing a documented auto trait requires a
major release. Compatible additions may be made in a minor release.

`Database`, immutable read transactions, options, batches, iterators, IDs,
reports, statistics, and errors are `Send + Sync` where the compile contract
asserts that property. A live `WriteTransaction` deliberately carries the
single-writer guard and has no cross-thread auto-trait promise.

Errors and returned report/statistics structures are marked `#[non_exhaustive]`.
Applications may inspect their documented variants and fields, but must retain
a wildcard arm when matching errors and must not construct result structures by
literal. This permits new diagnostic variants and counters without weakening
existing behavior or requiring a major release.

The persistence format has its own compatibility contract in
[File format](file-format.md). Rust API compatibility does not imply that the
unreleased file bytes have already been frozen.
