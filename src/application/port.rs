use std::path::Path;
use std::sync::Arc;

use crate::Result;
use crate::domain::{ChangeSet, Head, VerificationReport};
use crate::{
    BackupReport, CheckpointStats, CompactionReport, Error, OperationalStats, PageCacheStats,
    TransactionId,
};

pub(crate) struct CompactedStorage {
    pub(crate) store: Arc<dyn StorageEngine>,
    pub(crate) head: Head,
    pub(crate) report: CompactionReport,
}

impl CompactedStorage {
    pub(crate) fn require_consistent_report(self, source: Head) -> Result<Self> {
        if self.report.source_transaction_id != source.transaction_id()
            || self.report.compacted_snapshot.transaction_id != self.head.transaction_id()
            || self.report.compacted_snapshot.entries != self.head.entry_count()
        {
            return Err(Error::InternalInvariant(
                "active compaction returned an inconsistent report",
            ));
        }
        Ok(self)
    }
}

pub(crate) struct RefreshedStorage {
    pub(crate) store: Option<Arc<dyn StorageEngine>>,
    pub(crate) head: Head,
}

pub(crate) struct CommittedStorage {
    pub(crate) store: Option<Arc<dyn StorageEngine>>,
    pub(crate) head: Head,
}

impl CommittedStorage {
    pub(crate) fn require_transaction_id(self, expected: TransactionId) -> Result<Self> {
        if self.head.transaction_id() != expected {
            return Err(Error::InternalInvariant(
                "storage commit returned an unexpected transaction ID",
            ));
        }
        Ok(self)
    }

    pub(crate) fn require_logical_identity(self, expected: Head) -> Result<Self> {
        if self.head.transaction_id() != expected.transaction_id()
            || self.head.entry_count() != expected.entry_count()
        {
            return Err(Error::InternalInvariant(
                "storage maintenance changed the logical head",
            ));
        }
        Ok(self)
    }
}

pub(crate) trait StorageEngine: Send + Sync {
    fn get(&self, head: Head, key: &[u8]) -> Result<Option<Vec<u8>>>;

    fn scan(&self, head: Head) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;

    fn scan_range(
        &self,
        head: Head,
        start: &[u8],
        start_exclusive: bool,
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;

    fn verify(&self, head: Head) -> Result<VerificationReport>;

    fn page_cache_stats(&self) -> Result<PageCacheStats>;

    fn checkpoint_stats(&self) -> Result<CheckpointStats>;

    fn backup(&self, target: &Path) -> Result<BackupReport>;

    fn compact_to(&self, target: &Path) -> Result<CompactionReport>;

    fn flush(&self, head: Head) -> Result<CommittedStorage>;

    fn compact_active(&self, head: Head) -> Result<CompactedStorage>;

    fn refresh(&self) -> Result<RefreshedStorage>;

    fn operational_stats(&self) -> Result<OperationalStats>;

    fn commit(&self, base: Head, changes: ChangeSet) -> Result<CommittedStorage>;

    fn commit_group(&self, base: Head, changes: Vec<ChangeSet>) -> Result<CommittedStorage>;
}
