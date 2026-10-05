use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::Duration;

use super::{Database, validate_group_limit};
use crate::application::{CommittedStorage, CompactedStorage, RefreshedStorage, StorageEngine};
use crate::domain::{ChangeSet, Head};
use crate::{
    BackupReport, CheckpointStats, CompactionReport, Error, MAX_GROUP_TRANSACTIONS,
    OperationalStats, PageCacheStats, Result, TransactionId, VerificationReport, WriteBatch,
};

#[test]
fn group_transaction_limit_is_checked_without_allocating_batches() {
    assert!(validate_group_limit(MAX_GROUP_TRANSACTIONS).is_ok());
    assert!(matches!(
        validate_group_limit(MAX_GROUP_TRANSACTIONS + 1),
        Err(Error::InvalidInput(
            "group commit exceeds the transaction limit"
        ))
    ));
}

struct BlockingVerificationStore {
    entered: Barrier,
    release: Barrier,
    commit_advance: usize,
    refresh_head: Head,
    block_verification: bool,
    maintenance_head: Option<Head>,
    commit_calls: AtomicUsize,
}

impl BlockingVerificationStore {
    fn new() -> Self {
        Self {
            entered: Barrier::new(2),
            release: Barrier::new(2),
            commit_advance: 1,
            refresh_head: Head::empty(),
            block_verification: true,
            maintenance_head: None,
            commit_calls: AtomicUsize::new(0),
        }
    }

    fn with_commit_advance(commit_advance: usize) -> Self {
        Self {
            entered: Barrier::new(2),
            release: Barrier::new(2),
            commit_advance,
            refresh_head: Head::empty(),
            block_verification: false,
            maintenance_head: None,
            commit_calls: AtomicUsize::new(0),
        }
    }

    fn with_refresh_head(refresh_head: Head) -> Self {
        Self {
            entered: Barrier::new(2),
            release: Barrier::new(2),
            commit_advance: 1,
            refresh_head,
            block_verification: false,
            maintenance_head: None,
            commit_calls: AtomicUsize::new(0),
        }
    }

    fn with_maintenance_head(maintenance_head: Head) -> Self {
        Self {
            entered: Barrier::new(2),
            release: Barrier::new(2),
            commit_advance: 1,
            refresh_head: Head::empty(),
            block_verification: false,
            maintenance_head: Some(maintenance_head),
            commit_calls: AtomicUsize::new(0),
        }
    }

    fn committed_head(&self, base: Head) -> Result<Head> {
        let mut transaction_id = base.transaction_id();
        for _ in 0..self.commit_advance {
            transaction_id = transaction_id.next()?;
        }
        Ok(Head::new(
            transaction_id,
            base.root(),
            base.last_page(),
            base.entry_count(),
        ))
    }
}

impl StorageEngine for BlockingVerificationStore {
    fn get(&self, _head: Head, _key: &[u8]) -> Result<Option<Vec<u8>>> {
        unused()
    }

    fn scan(&self, _head: Head) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        unused()
    }

    fn scan_range(
        &self,
        _head: Head,
        _start: &[u8],
        _start_exclusive: bool,
        _end: Option<&[u8]>,
        _limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        unused()
    }

    fn verify(&self, head: Head) -> Result<VerificationReport> {
        if self.block_verification {
            self.entered.wait();
            self.release.wait();
        }
        Ok(VerificationReport {
            transaction_id: head.transaction_id(),
            entries: head.entry_count(),
            tree_pages: 0,
            overflow_pages: 0,
            tree_depth: 0,
            logical_hash: [0; 32],
        })
    }

    fn page_cache_stats(&self) -> Result<PageCacheStats> {
        unused()
    }

    fn checkpoint_stats(&self) -> Result<CheckpointStats> {
        unused()
    }

    fn backup(&self, _target: &Path) -> Result<BackupReport> {
        unused()
    }

    fn compact_to(&self, _target: &Path) -> Result<CompactionReport> {
        unused()
    }

    fn flush(&self, head: Head) -> Result<CommittedStorage> {
        Ok(CommittedStorage {
            store: None,
            head: self.maintenance_head.unwrap_or(head),
        })
    }

    fn compact_active(&self, head: Head) -> Result<CompactedStorage> {
        let compacted_head = self.maintenance_head.unwrap_or(head);
        Ok(CompactedStorage {
            store: Arc::new(Self::with_refresh_head(compacted_head)),
            head: compacted_head,
            report: CompactionReport {
                source_transaction_id: head.transaction_id(),
                compacted_snapshot: VerificationReport {
                    transaction_id: head.transaction_id(),
                    entries: head.entry_count(),
                    tree_pages: 0,
                    overflow_pages: 0,
                    tree_depth: 0,
                    logical_hash: [0; 32],
                },
                source_bytes: 0,
                compacted_bytes: 0,
                publication_pause: Duration::ZERO,
            },
        })
    }

    fn refresh(&self) -> Result<RefreshedStorage> {
        Ok(RefreshedStorage {
            store: None,
            head: self.refresh_head,
        })
    }

    fn operational_stats(&self) -> Result<OperationalStats> {
        unused()
    }

    fn commit(&self, base: Head, _changes: ChangeSet) -> Result<CommittedStorage> {
        self.commit_calls.fetch_add(1, Ordering::Relaxed);
        Ok(CommittedStorage {
            store: None,
            head: self.committed_head(base)?,
        })
    }

    fn commit_group(&self, base: Head, _changes: Vec<ChangeSet>) -> Result<CommittedStorage> {
        self.commit_calls.fetch_add(1, Ordering::Relaxed);
        Ok(CommittedStorage {
            store: None,
            head: self.committed_head(base)?,
        })
    }
}

fn unused<T>() -> Result<T> {
    Err(Error::InternalInvariant("unused test storage operation"))
}

#[test]
fn transaction_id_exhaustion_rejects_commit_before_storage() {
    let store = Arc::new(BlockingVerificationStore::new());
    let exhausted = Head::new(TransactionId::from_raw(u64::MAX), None, 1, 0);
    let database = Database::from_store(store.clone(), exhausted);
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"value").unwrap();

    assert!(matches!(
        writer.commit(),
        Err(Error::TransactionIdExhausted)
    ));
    assert_eq!(store.commit_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        database.stats().unwrap().transaction_id,
        exhausted.transaction_id()
    );
}

#[test]
fn group_transaction_id_exhaustion_rejects_every_batch_before_storage() {
    let store = Arc::new(BlockingVerificationStore::new());
    let penultimate = Head::new(TransactionId::from_raw(u64::MAX - 1), None, 1, 0);
    let database = Database::from_store(store.clone(), penultimate);

    assert!(matches!(
        database.commit_group(vec![WriteBatch::new(), WriteBatch::new()]),
        Err(Error::TransactionIdExhausted)
    ));
    assert_eq!(store.commit_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        database.stats().unwrap().transaction_id,
        penultimate.transaction_id()
    );
}

#[test]
fn automatic_batch_transaction_id_exhaustion_releases_leadership_for_the_next_call() {
    let store = Arc::new(BlockingVerificationStore::new());
    let exhausted = Head::new(TransactionId::from_raw(u64::MAX), None, 1, 0);
    let database = Database::from_store(store.clone(), exhausted);

    for _ in 0..2 {
        assert!(matches!(
            database.commit_batch(WriteBatch::new()),
            Err(Error::TransactionIdExhausted)
        ));
    }
    assert_eq!(store.commit_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        database.stats().unwrap().transaction_id,
        exhausted.transaction_id()
    );
}

#[test]
fn refresh_rejects_a_transaction_regression_without_changing_the_snapshot() {
    let store = Arc::new(BlockingVerificationStore::new());
    let current = Head::new(TransactionId::from_raw(1), None, 1, 0);
    let database = Database::from_store(store, current);

    assert!(matches!(database.refresh(), Err(Error::WriteConflict)));
    assert_eq!(
        database.stats().unwrap().transaction_id,
        current.transaction_id()
    );
}

#[test]
fn refresh_verifies_a_changed_same_transaction_head_in_the_current_store() {
    let transaction_id = TransactionId::from_raw(1);
    let current = Head::new(transaction_id, None, 1, 1);
    let divergent = Head::new(transaction_id, None, 1, 2);
    let store = Arc::new(BlockingVerificationStore::with_refresh_head(divergent));
    let database = Database::from_store(store, current);

    assert!(matches!(database.refresh(), Err(Error::WriteConflict)));
    let stats = database.stats().unwrap();
    assert_eq!(stats.transaction_id, transaction_id);
    assert_eq!(stats.entries, 1);
}

#[test]
fn refresh_accepts_an_equivalent_same_transaction_head_in_the_current_store() {
    let transaction_id = TransactionId::from_raw(1);
    let current = Head::new(transaction_id, None, 1, 1);
    let equivalent = Head::new(transaction_id, None, 2, 1);
    let store = Arc::new(BlockingVerificationStore::with_refresh_head(equivalent));
    let database = Database::from_store(store, current);

    assert_eq!(database.refresh().unwrap(), transaction_id);
    let stats = database.stats().unwrap();
    assert_eq!(stats.transaction_id, transaction_id);
    assert_eq!(stats.entries, 1);
    assert_eq!(stats.last_page, 2);
}

#[test]
fn long_snapshot_read_does_not_block_commit_publication() {
    let store = Arc::new(BlockingVerificationStore::new());
    let database = Database::from_store(store.clone(), Head::empty());

    let verifying = database.clone();
    let verify_thread = thread::spawn(move || verifying.verify());
    store.entered.wait();

    let committing = database.clone();
    let (completed, completion) = mpsc::channel();
    let commit_thread = thread::spawn(move || {
        let result = (|| {
            let mut writer = committing.write()?;
            writer.put(b"key", b"value")?;
            writer.commit()
        })();
        completed.send(result).expect("commit receiver is alive");
    });

    let early = completion.recv_timeout(Duration::from_secs(2));
    let completed_before_release = early.is_ok();
    store.release.wait();
    verify_thread
        .join()
        .expect("verify thread panicked")
        .unwrap();
    let committed = match early {
        Ok(result) => result,
        Err(_) => completion
            .recv_timeout(Duration::from_secs(2))
            .expect("commit did not finish after verification was released"),
    };
    commit_thread.join().expect("commit thread panicked");

    committed.unwrap();
    assert!(
        completed_before_release,
        "commit publication waited for an unrelated snapshot read"
    );
    assert_eq!(database.stats().unwrap().transaction_id.value(), 1);
}

#[test]
fn commit_rejects_an_unexpected_storage_transaction_id() {
    let store = Arc::new(BlockingVerificationStore::with_commit_advance(2));
    let database = Database::from_store(store, Head::empty());
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"value").unwrap();

    assert!(matches!(
        writer.commit(),
        Err(Error::InternalInvariant(
            "storage commit returned an unexpected transaction ID"
        ))
    ));
    assert_eq!(database.stats().unwrap().transaction_id.value(), 0);
}

#[test]
fn group_commit_rejects_an_unexpected_storage_transaction_id() {
    let store = Arc::new(BlockingVerificationStore::new());
    let database = Database::from_store(store, Head::empty());

    assert!(matches!(
        database.commit_group(vec![Default::default(), Default::default()]),
        Err(Error::InternalInvariant(
            "storage commit returned an unexpected transaction ID"
        ))
    ));
    assert_eq!(database.stats().unwrap().transaction_id.value(), 0);
}

#[test]
fn flush_rejects_a_changed_logical_head() {
    let current = Head::new(TransactionId::from_raw(1), None, 1, 1);
    let changed = Head::new(TransactionId::from_raw(1), None, 2, 2);
    let store = Arc::new(BlockingVerificationStore::with_maintenance_head(changed));
    let database = Database::from_store(store, current);

    assert!(matches!(
        database.flush(),
        Err(Error::InternalInvariant(
            "storage maintenance changed the logical head"
        ))
    ));
    assert_eq!(database.stats().unwrap().entries, 1);
}

#[test]
fn active_compaction_rejects_an_inconsistent_report() {
    let current = Head::new(TransactionId::from_raw(1), None, 1, 1);
    let changed = Head::new(TransactionId::from_raw(2), None, 2, 2);
    let store = Arc::new(BlockingVerificationStore::with_maintenance_head(changed));
    let database = Database::from_store(store, current);

    assert!(matches!(
        database.compact(),
        Err(Error::InternalInvariant(
            "active compaction returned an inconsistent report"
        ))
    ));
    let stats = database.stats().unwrap();
    assert_eq!(stats.transaction_id.value(), 1);
    assert_eq!(stats.entries, 1);
}
