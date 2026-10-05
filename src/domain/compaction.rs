use std::time::Duration;

use super::{TransactionId, VerificationReport};

/// Result of rebuilding one snapshot into a compact database file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CompactionReport {
    /// Transaction identifier of the source snapshot.
    pub source_transaction_id: TransactionId,
    /// Full verification result for the compacted file.
    pub compacted_snapshot: VerificationReport,
    /// Physical bytes in the source when compaction started.
    pub source_bytes: u64,
    /// Physical bytes in the compacted database and retained write buffer.
    pub compacted_bytes: u64,
    /// Time for which active compaction held the publication locks.
    ///
    /// This includes the WAL generation handoff and atomic main-file
    /// replacement. Snapshot compaction into a separate target reports zero
    /// because it does not pause foreground writers.
    pub publication_pause: Duration,
}
