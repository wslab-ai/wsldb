use super::TransactionId;

/// Result of a complete validation of one committed database snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct VerificationReport {
    /// Identifier of the snapshot that was verified.
    pub transaction_id: TransactionId,
    /// Number of validated key/value entries.
    pub entries: u64,
    /// Number of reachable branch and leaf pages.
    pub tree_pages: u64,
    /// Number of reachable overflow pages.
    pub overflow_pages: u64,
    /// Number of tree levels, or zero for an empty database.
    pub tree_depth: u32,
    /// SHA-256 of the canonical ordered key/value stream.
    pub logical_hash: [u8; 32],
}
