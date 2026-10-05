use std::path::Path;
use std::sync::{Arc, RwLock};

use super::{BufferedWrites, ReplacementResources};
use crate::application::StorageEngine;
use crate::domain::Head;
use crate::infrastructure::file_context::load_head;
use crate::infrastructure::file_store::FileStorageEngine;
use crate::infrastructure::membership_filter::GrowingMembershipFilter;
use crate::infrastructure::write_buffer::{PointIndexSlot, WriteBuffer};
use crate::infrastructure::{wal, wal::WalRecord};
use crate::{Error, Result};

pub(super) fn open(
    path: &Path,
    checkpoint: Head,
    threshold: u64,
) -> Result<(Option<BufferedWrites>, Head)> {
    if threshold == 0 && wal::file_len(path)? == 0 {
        return Ok((None, checkpoint));
    }
    let file = wal::open(path)?;
    let key_filter = super::new_buffer_key_filter(threshold)?;
    let mut recovery = WalRecovery::new(checkpoint, Some(key_filter));
    let scan = wal::replay(&file, |record| recovery.consume(record))?;
    let mut valid_bytes = scan.valid_bytes;
    if scan.has_incomplete_tail {
        wal::truncate(&file, valid_bytes)?;
    }
    if recovery.can_discard_wal() {
        wal::truncate(&file, 0)?;
        valid_bytes = 0;
    }
    if threshold == 0 && valid_bytes == 0 {
        return Ok((None, checkpoint));
    }

    let mut state = recovery.finish();
    state.set_wal_bytes(valid_bytes);
    let head = state.head();
    Ok((
        Some(BufferedWrites {
            file,
            state: RwLock::new(state),
            checkpoint,
            point_index: Arc::new(PointIndexSlot::new()),
            membership_filter: None,
            physical_checkpoint: RwLock::new(checkpoint),
            base: RwLock::new(None),
            transition_epoch: std::sync::atomic::AtomicU64::new(0),
            main_may_have_advanced: std::sync::atomic::AtomicBool::new(false),
            checkpoint_task: None,
            wal_start: 0,
        }),
        head,
    ))
}

impl FileStorageEngine {
    pub(super) fn replacement_after_failed_checkpoint(
        &self,
        resources: ReplacementResources,
    ) -> Result<(Arc<dyn StorageEngine>, Head)> {
        let checkpoint = load_head(&resources.file)?;
        let (buffered_writes, head) =
            Self::open_buffer(&self.path, checkpoint, self.write_buffer_bytes)?;
        Ok((
            Arc::new(Self {
                file: resources.file,
                path: self.path.clone(),
                lock_file: resources.lock_file,
                commit_lock: Arc::clone(&self.commit_lock),
                page_cache: resources.page_cache,
                write_buffer_bytes: self.write_buffer_bytes,
                buffered_writes: buffered_writes.map(Arc::new),
                recovery_view: None,
                metrics: Arc::clone(&self.metrics),
            }),
            head,
        ))
    }
}

/// Reconciles a durable checkpoint with a WAL that may still contain the
/// prefix used to build that checkpoint. Keeping the prefix valid until after
/// metadata publication makes the two-file transition crash safe.
struct WalRecovery {
    checkpoint: Head,
    state: WriteBuffer,
    saw_record: bool,
    reached_checkpoint: bool,
}

impl WalRecovery {
    fn new(checkpoint: Head, key_filter: Option<GrowingMembershipFilter>) -> Self {
        let state = key_filter.map_or_else(
            || WriteBuffer::empty(checkpoint),
            |filter| WriteBuffer::empty_with_membership_filter(checkpoint, filter),
        );
        Self {
            checkpoint,
            state,
            saw_record: false,
            reached_checkpoint: false,
        }
    }

    fn consume(&mut self, record: WalRecord) -> Result<()> {
        self.saw_record = true;
        let checkpoint_id = self.checkpoint.transaction_id();
        if record.transaction_id < checkpoint_id {
            if self.reached_checkpoint {
                return Err(discontinuous_checkpoint());
            }
            return Ok(());
        }
        if record.transaction_id == checkpoint_id {
            if self.reached_checkpoint || record.entry_count != self.checkpoint.entry_count() {
                return Err(Error::Corrupt(
                    "checkpoint disagrees with its write-ahead log".into(),
                ));
            }
            self.reached_checkpoint = true;
            return Ok(());
        }
        if !self.reached_checkpoint {
            if record.base_transaction_id != checkpoint_id {
                return Err(discontinuous_checkpoint());
            }
            self.reached_checkpoint = true;
        }
        self.state.replay(record)
    }

    fn can_discard_wal(&self) -> bool {
        self.saw_record && self.state.head() == self.checkpoint
    }

    fn finish(self) -> WriteBuffer {
        self.state
    }
}

fn discontinuous_checkpoint() -> Error {
    Error::Corrupt("write-ahead log does not contain its checkpoint boundary".into())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{WalRecovery, open};
    use crate::domain::{ChangeSet, Head};
    use crate::infrastructure::wal::{self, WalRecord, WalRecordKind};
    use crate::{Error, TransactionId};

    static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn opening_a_wal_with_a_checkpoint_prefix_replays_its_tail() {
        let path = temporary_path("checkpoint-prefix");
        let file = wal::open(&path).unwrap();
        let mut end = 0;
        for next in 1..=3 {
            end = wal::append(
                &file,
                end,
                TransactionId::from_raw(next - 1),
                TransactionId::from_raw(next),
                1,
                &one_change(next),
            )
            .unwrap()
            .end;
        }
        drop(file);

        let (buffer, recovered) = open(&path, head(2, 1), 1024).unwrap();
        assert_eq!(recovered.transaction_id().value(), 3);
        assert_eq!(recovered.entry_count(), 1);
        assert_eq!(
            buffer.unwrap().file.metadata().unwrap().len(),
            end,
            "the prefix remains durable until a later WAL consolidation"
        );

        remove_wal(&path);
    }

    #[test]
    fn replays_only_the_tail_after_a_published_checkpoint() {
        let checkpoint = head(2, 1);
        let mut recovery = WalRecovery::new(checkpoint, None);
        recovery.consume(record(0, 1, 1)).unwrap();
        recovery.consume(record(1, 2, 1)).unwrap();
        recovery.consume(record(2, 3, 2)).unwrap();

        assert_eq!(recovery.state.checkpoint(), checkpoint);
        assert_eq!(recovery.state.head().transaction_id().value(), 3);
        assert_eq!(recovery.state.head().entry_count(), 2);
        assert!(!recovery.can_discard_wal());
    }

    #[test]
    fn discards_a_prefix_fully_covered_by_the_checkpoint() {
        let checkpoint = head(2, 1);
        let mut recovery = WalRecovery::new(checkpoint, None);
        recovery.consume(record(0, 1, 1)).unwrap();
        recovery.consume(record(1, 2, 1)).unwrap();

        assert!(recovery.can_discard_wal());
        assert_eq!(recovery.state.head(), checkpoint);
    }

    #[test]
    fn rejects_a_chain_that_jumps_over_the_checkpoint() {
        let mut recovery = WalRecovery::new(head(2, 1), None);
        let error = recovery.consume(snapshot(0, 3, 2)).unwrap_err();

        assert!(matches!(error, Error::Corrupt(_)));
    }

    #[test]
    fn rejects_a_checkpoint_entry_count_disagreement() {
        let mut recovery = WalRecovery::new(head(2, 9), None);
        recovery.consume(record(0, 1, 1)).unwrap();
        let error = recovery.consume(record(1, 2, 1)).unwrap_err();

        assert!(matches!(error, Error::Corrupt(_)));
    }

    fn head(transaction: u64, entries: u64) -> Head {
        Head::new(TransactionId::from_raw(transaction), None, 1, entries)
    }

    fn record(base: u64, transaction: u64, entries: u64) -> WalRecord {
        WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(base),
            transaction_id: TransactionId::from_raw(transaction),
            entry_count: entries,
            changes: one_change(transaction),
        }
    }

    fn snapshot(base: u64, transaction: u64, entries: u64) -> WalRecord {
        WalRecord {
            kind: WalRecordKind::Snapshot,
            base_transaction_id: TransactionId::from_raw(base),
            transaction_id: TransactionId::from_raw(transaction),
            entry_count: entries,
            changes: one_change(transaction),
        }
    }

    fn one_change(transaction: u64) -> ChangeSet {
        let mut changes = ChangeSet::new();
        changes.insert(b"key", Some(transaction.to_le_bytes().to_vec()));
        changes
    }

    fn temporary_path(label: &str) -> PathBuf {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "wsldb-{}-{sequence}-{label}.recovery-test",
            std::process::id()
        ))
    }

    fn remove_wal(database: &Path) {
        let path = wal::path(database);
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove recovery test WAL: {error}"),
        }
    }
}
