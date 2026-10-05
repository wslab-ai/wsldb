use super::VerificationReport;

/// Result of creating and validating a durable database backup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BackupReport {
    /// Full verification result for the copied snapshot.
    pub snapshot: VerificationReport,
    /// Number of bytes copied through the committed page boundary.
    pub bytes_copied: u64,
}
