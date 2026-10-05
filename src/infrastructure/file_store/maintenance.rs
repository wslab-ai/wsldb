use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use super::{FileStorageEngine, RecoveryView};
use crate::application::{CompactedStorage, StorageEngine};
use crate::domain::{ChangeSet, Head};
use crate::infrastructure::file_context::{ReadContext, UncachedReadContext, load_head};
use crate::infrastructure::metadata::PAGE_SIZE;
use crate::infrastructure::path_lock::{
    AdvisoryFileLock, MainFileLease, open_database, open_lock_file, replace_file,
    same_file_identity,
};
use crate::infrastructure::{
    compaction, crash_injection, fault_injection, snapshot_file, tree, wal,
};
use crate::{BackupReport, CompactionReport, Error, Result};

struct BufferedMaintenanceSnapshot {
    physical_head: Head,
    logical_head: Head,
    changes: ChangeSet,
    source_bytes: u64,
    wal_bytes: u64,
}

impl FileStorageEngine {
    fn capture_recovery_source(&self, view: RecoveryView) -> Result<(std::fs::File, u64)> {
        let _guard = self.lock_commit()?;
        let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
        self.reject_external_buffer()?;
        let source = open_database(&self.path)?;
        if !same_file_identity(&self.file, &source)? || load_head(&source)? != view.physical_head {
            return Err(Error::WriteConflict);
        }
        let source_bytes = source.metadata()?.len();
        Ok((source, source_bytes))
    }

    fn capture_buffered_maintenance(
        &self,
        buffer: &super::BufferedWrites,
        expected: Option<Head>,
    ) -> Result<BufferedMaintenanceSnapshot> {
        let state = Self::buffer_state(buffer)?;
        let logical_head = state.head();
        let physical_head = buffer.physical_checkpoint()?;
        let file = open_database(&self.path)?;
        if expected.is_some_and(|expected| expected != logical_head)
            || !same_file_identity(&self.file, &file)?
            || load_head(&file)? != physical_head
            || !self.buffer_file_matches(buffer, &state)?
        {
            return Err(Error::WriteConflict);
        }
        let wal_bytes = state.wal_bytes();
        let source_bytes = self
            .file
            .metadata()?
            .len()
            .checked_add(wal_bytes)
            .ok_or(Error::FileTooLarge)?;
        drop(state);
        let changes = buffer.layered_latest_changes(logical_head)?;
        Ok(BufferedMaintenanceSnapshot {
            physical_head,
            logical_head,
            changes,
            source_bytes,
            wal_bytes,
        })
    }

    pub(crate) fn compact_file(path: &Path) -> Result<CompactionReport> {
        let (store, head) = Self::open(path, 0, 0, false)?;
        store.compact_active(head).map(|compacted| compacted.report)
    }

    fn compact_file_from(path: &Path, expected: Option<Head>) -> Result<CompactionReport> {
        let _main_file_lease = MainFileLease::acquire(path)?;
        let lock_file = open_lock_file(path)?;
        let (source, source_head, source_bytes) = {
            let _file_lock = AdvisoryFileLock::acquire(&lock_file)?;
            if wal::file_len(path)? != 0 {
                return Err(Error::WriteConflict);
            }
            let source = open_database(path)?;
            let source_head = load_head(&source)?;
            if expected.is_some_and(|expected| expected != source_head) {
                return Err(Error::WriteConflict);
            }
            let source_bytes = source.metadata()?.len();
            (source, source_head, source_bytes)
        };
        let replacement = snapshot_file::replacement_path(path)?;
        let transaction_id = source_head.transaction_id().next()?;
        let result = compaction::compact(
            &UncachedReadContext::new(&source, source_head),
            source_head,
            transaction_id,
            source_bytes,
            &replacement,
        );
        let mut report = result?;
        crash_injection::hit("compaction_ready");

        let publication_started = Instant::now();
        let publish = (|| {
            let _file_lock = AdvisoryFileLock::acquire(&lock_file)?;
            if wal::file_len(path)? != 0 {
                return Err(Error::WriteConflict);
            }
            let current = open_database(path)?;
            if !same_file_identity(&source, &current)? || load_head(&current)? != source_head {
                return Err(Error::WriteConflict);
            }
            replace_file(&replacement, path)
        })();
        if publish.is_err() {
            let _ = std::fs::remove_file(&replacement);
        }
        publish.map(|()| {
            report.publication_pause = publication_started.elapsed();
            report
        })
    }

    pub(super) fn backup_to(&self, target: &Path) -> Result<BackupReport> {
        if let Some(view) = self.recovery_view {
            let (source, _) = self.capture_recovery_source(view)?;
            return Self::backup_snapshot(source, view.head, target);
        }
        if let Some(buffer) = &self.buffered_writes {
            let snapshot = {
                let _guard = self.lock_commit()?;
                let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
                self.capture_buffered_maintenance(buffer, None)?
            };
            let report = compaction::compact_layered(
                &ReadContext::new(&self.file, &self.page_cache, snapshot.physical_head),
                snapshot.physical_head,
                snapshot.logical_head,
                snapshot.changes,
                snapshot.logical_head.transaction_id(),
                snapshot.source_bytes,
                target,
            )?;
            return Ok(BackupReport {
                snapshot: report.compacted_snapshot,
                bytes_copied: report.compacted_bytes,
            });
        }
        let (source, head) = {
            let _guard = self.lock_commit()?;
            let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
            self.reject_external_buffer()?;
            let source = open_database(&self.path)?;
            let head = load_head(&source)?;
            (source, head)
        };
        Self::backup_snapshot(source, head, target)
    }

    fn backup_snapshot(source: std::fs::File, head: Head, target: &Path) -> Result<BackupReport> {
        let committed_bytes = head
            .last_page()
            .checked_add(1)
            .and_then(|pages| pages.checked_mul(PAGE_SIZE as u64))
            .ok_or(Error::FileTooLarge)?;
        let (destination, temporary) = snapshot_file::create_temporary(target)?;
        snapshot_file::copy(&source, &destination, head, committed_bytes)?;
        fault_injection::before("backup_sync")?;
        destination.sync_all()?;
        crash_injection::hit("backup_sync");

        let copied_head = load_head(&destination)?;
        if copied_head != head {
            return Err(Error::Corrupt(
                "backup metadata does not match its source snapshot".into(),
            ));
        }
        let snapshot = tree::verify(
            &UncachedReadContext::new(&destination, copied_head),
            copied_head,
        )?;
        snapshot_file::publish(destination, temporary, target)?;
        Ok(BackupReport {
            snapshot,
            bytes_copied: committed_bytes,
        })
    }

    pub(super) fn compact_to_path(&self, target: &Path) -> Result<CompactionReport> {
        if let Some(view) = self.recovery_view {
            let (source, source_bytes) = self.capture_recovery_source(view)?;
            return compaction::compact(
                &UncachedReadContext::new(&source, view.head),
                view.head,
                view.head.transaction_id(),
                source_bytes,
                target,
            );
        }
        if let Some(buffer) = &self.buffered_writes {
            let snapshot = {
                let _guard = self.lock_commit()?;
                let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
                self.capture_buffered_maintenance(buffer, None)?
            };
            return compaction::compact_layered(
                &ReadContext::new(&self.file, &self.page_cache, snapshot.physical_head),
                snapshot.physical_head,
                snapshot.logical_head,
                snapshot.changes,
                snapshot.logical_head.transaction_id(),
                snapshot.source_bytes,
                target,
            );
        }
        let (source, head, source_bytes) = {
            let _guard = self.lock_commit()?;
            let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
            self.reject_external_buffer()?;
            let source = open_database(&self.path)?;
            let head = load_head(&source)?;
            let source_bytes = source.metadata()?.len();
            (source, head, source_bytes)
        };
        compaction::compact(
            &UncachedReadContext::new(&source, head),
            head,
            head.transaction_id(),
            source_bytes,
            target,
        )
    }

    pub(super) fn compact_active_store(&self, head: Head) -> Result<CompactedStorage> {
        let capacity = self.page_cache.stats()?.capacity;
        if let Some(buffer) = &self.buffered_writes {
            let _main_file_lease = MainFileLease::acquire(&self.path)?;
            let snapshot = {
                let _guard = self.lock_commit()?;
                let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
                self.capture_buffered_maintenance(buffer, Some(head))?
            };
            let source_head = snapshot.logical_head;
            let source_wal_bytes = snapshot.wal_bytes;
            let replacement = snapshot_file::replacement_path(&self.path)?;
            let result = compaction::compact_layered(
                &ReadContext::new(&self.file, &self.page_cache, snapshot.physical_head),
                snapshot.physical_head,
                source_head,
                snapshot.changes,
                source_head.transaction_id(),
                snapshot.source_bytes,
                &replacement,
            );
            let mut report = result?;
            crash_injection::hit("compaction_ready");
            let handoff = match wal::WalHandoff::stage(&self.path, &buffer.file, source_wal_bytes) {
                Ok(handoff) => handoff,
                Err(error) => {
                    let _ = std::fs::remove_file(&replacement);
                    return Err(error);
                }
            };
            let publication_started = Instant::now();
            let publish = (|| {
                let _guard = self.lock_commit()?;
                let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
                let file = open_database(&self.path)?;
                let needs_refresh = {
                    let state = Self::buffer_state(buffer)?;
                    !self.buffer_file_matches(buffer, &state)?
                };
                if needs_refresh {
                    self.refresh_buffered_tail_locked(buffer, &file)?
                        .ok_or(Error::WriteConflict)?;
                }
                let state = Self::buffer_state(buffer)?;
                if (state.head().transaction_id() == source_head.transaction_id()
                    && state.head() != source_head)
                    || state.head().transaction_id() < source_head.transaction_id()
                    || state.wal_bytes() < source_wal_bytes
                    || !same_file_identity(&self.file, &file)?
                    || load_head(&file)? != buffer.physical_checkpoint()?
                    || !self.buffer_file_matches(buffer, &state)?
                {
                    return Err(Error::WriteConflict);
                }
                let current_head = state.head();
                let current_wal_bytes = state.wal_bytes();
                drop(state);
                handoff.publish(&buffer.file, current_wal_bytes)?;
                replace_file(&replacement, &self.path).map_err(super::publication_error)?;
                crash_injection::hit("buffered_compaction_publish");
                Ok((current_head, current_wal_bytes))
            })();
            report.publication_pause = publication_started.elapsed();
            if publish.is_err() {
                let _ = std::fs::remove_file(&replacement);
            }
            let (published_head, _) = publish?;
            let (store, compacted_head) = Self::open_with_metrics(
                &self.path,
                capacity,
                self.write_buffer_bytes,
                Arc::clone(&self.metrics),
                false,
            )?;
            if compacted_head.transaction_id() != published_head.transaction_id()
                || compacted_head.entry_count() != published_head.entry_count()
            {
                return Err(Error::InternalInvariant(
                    "compaction reopened a different snapshot",
                ));
            }
            report.compacted_snapshot = store.verify(compacted_head)?;
            report.compacted_bytes = std::fs::metadata(&self.path)?
                .len()
                .checked_add(wal::file_len(&self.path)?)
                .ok_or(Error::FileTooLarge)?;
            return Ok(CompactedStorage {
                store: Arc::new(store),
                head: compacted_head,
                report,
            });
        }
        let report = Self::compact_file_from(&self.path, Some(head))?;
        let (store, compacted_head) = Self::open_with_metrics(
            &self.path,
            capacity,
            self.write_buffer_bytes,
            Arc::clone(&self.metrics),
            false,
        )?;
        Ok(CompactedStorage {
            store: Arc::new(store),
            head: compacted_head,
            report,
        })
    }
}
