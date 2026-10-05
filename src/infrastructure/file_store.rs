use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

mod buffered;
use buffered::BufferedWrites;
mod maintenance;

use super::crash_injection;
use super::cursor;
use super::fault_injection;
use super::file_context::{
    CommitContext, ReadContext, UncachedReadContext, load_head, load_head_candidates,
    next_page_after,
};
use super::initialization;
use super::metadata::{self, METADATA_SLOTS, PAGE_SIZE};
use super::metrics::RuntimeMetrics;
use super::page_cache::PageCache;
use super::path_lock::{
    AdvisoryFileLock, LockFile, normalize_database_path, open_database, open_lock_file,
    same_file_identity, validate_database_identity,
};
use super::positioned_io::write_all_at;
use super::tree;
use super::tree_verify;
use super::wal;
use crate::application::{CommittedStorage, CompactedStorage, RefreshedStorage, StorageEngine};
use crate::domain::{ChangeSet, Head, VerificationReport};
use crate::{
    BackupReport, CheckpointStats, CompactionReport, Error, OperationalStats, PageCacheStats,
    Result,
};

const RECOVERED_WRITE_BUFFER_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) struct FileStorageEngine {
    file: File,
    path: PathBuf,
    lock_file: LockFile,
    commit_lock: Arc<Mutex<()>>,
    page_cache: Arc<PageCache>,
    write_buffer_bytes: u64,
    buffered_writes: Option<Arc<BufferedWrites>>,
    recovery_view: Option<RecoveryView>,
    metrics: Arc<RuntimeMetrics>,
}

#[derive(Clone, Copy)]
struct RecoveryView {
    head: Head,
    physical_head: Head,
}

fn publication_error(error: Error) -> Error {
    match error {
        Error::Io(error) => Error::PublicationOutcomeUnknown(error),
        other => other,
    }
}

impl FileStorageEngine {
    pub(crate) fn open(
        path: &Path,
        page_cache_capacity: usize,
        write_buffer_bytes: u64,
        verify_on_open: bool,
    ) -> Result<(Self, Head)> {
        Self::open_with_metrics(
            path,
            page_cache_capacity,
            write_buffer_bytes,
            Arc::new(RuntimeMetrics::default()),
            verify_on_open,
        )
    }

    fn open_with_metrics(
        path: &Path,
        page_cache_capacity: usize,
        write_buffer_bytes: u64,
        metrics: Arc<RuntimeMetrics>,
        verify_on_open: bool,
    ) -> Result<(Self, Head)> {
        let path = normalize_database_path(path)?;
        let lock_file = open_lock_file(&path)?;
        let _file_lock = AdvisoryFileLock::acquire(&lock_file)?;
        let initialized = initialization::open_or_create(&path)?;
        validate_database_identity(&path, &initialized.file)?;
        let verification_candidates = if verify_on_open {
            Some(load_head_candidates(&initialized.file)?)
        } else {
            None
        };
        let wal_was_empty = !verify_on_open || wal::file_len(&path)? == 0;
        let mut store = Self {
            file: initialized.file,
            path: path.clone(),
            lock_file: lock_file.try_clone()?,
            commit_lock: Arc::new(Mutex::new(())),
            page_cache: Arc::new(PageCache::new(page_cache_capacity)),
            write_buffer_bytes,
            buffered_writes: None,
            recovery_view: None,
            metrics,
        };

        let selected_head = initialized.head;
        let (buffered_writes, mut head) =
            Self::open_buffer(&path, selected_head, write_buffer_bytes)?;
        store.buffered_writes = buffered_writes.map(Arc::new);
        if verify_on_open && let Err(error) = store.verify(head) {
            if !wal_was_empty || !is_corruption_error(&error) {
                return Err(error);
            }
            let candidates = verification_candidates.ok_or(Error::InternalInvariant(
                "verified open candidates are missing",
            ))?;
            let mut recovered = None;
            for candidate in candidates.into_iter().rev() {
                if candidate == selected_head {
                    continue;
                }
                match tree::verify(&UncachedReadContext::new(&store.file, candidate), candidate) {
                    Ok(_) => {
                        recovered = Some(candidate);
                        break;
                    }
                    Err(candidate_error) if is_corruption_error(&candidate_error) => {}
                    Err(candidate_error) => return Err(candidate_error),
                }
            }
            let candidate = recovered.ok_or(error)?;
            let (buffered_writes, recovered_head) =
                Self::open_buffer(&path, candidate, write_buffer_bytes)?;
            if recovered_head != candidate {
                return Err(Error::InternalInvariant(
                    "empty WAL changed the verified fallback head",
                ));
            }
            store.buffered_writes = buffered_writes.map(Arc::new);
            store.recovery_view = Some(RecoveryView {
                head: candidate,
                physical_head: selected_head,
            });
            head = candidate;
        }
        Ok((store, head))
    }

    fn lock_commit(&self) -> Result<MutexGuard<'_, ()>> {
        self.commit_lock
            .lock()
            .map_err(|_| Error::LockPoisoned("file commit"))
    }

    fn observe<T>(&self, result: Result<T>) -> Result<T> {
        observe_corruption(&self.metrics, result)
    }

    fn reject_external_buffer(&self) -> Result<()> {
        if wal::file_len(&self.path)? != 0 {
            return Err(Error::WriteConflict);
        }
        Ok(())
    }
}

fn is_corruption_error(error: &Error) -> bool {
    matches!(error, Error::Corrupt(_) | Error::TreeDepthExceeded)
}

fn observe_corruption<T>(metrics: &RuntimeMetrics, result: Result<T>) -> Result<T> {
    if result.as_ref().is_err_and(is_corruption_error) {
        metrics.record_corruption();
    }
    result
}

impl StorageEngine for FileStorageEngine {
    fn get(&self, head: Head, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if let Some(buffer) = &self.buffered_writes {
            let result = buffer.stable_read(|| {
                if let Some(value) = buffer.layered_visible_value(head, key)? {
                    return Ok(value);
                }
                let checkpoint = buffer.physical_checkpoint()?;
                tree::get(
                    &ReadContext::new(&self.file, &self.page_cache, checkpoint),
                    checkpoint.root(),
                    key,
                )
            });
            return self.observe(result);
        }
        let result = tree::get(
            &ReadContext::new(&self.file, &self.page_cache, head),
            head.root(),
            key,
        );
        self.observe(result)
    }

    fn scan(&self, head: Head) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        if let Some(buffer) = &self.buffered_writes {
            let result = buffer.stable_read(|| {
                let checkpoint = buffer.physical_checkpoint()?;
                let base = cursor::scan(
                    &ReadContext::new(&self.file, &self.page_cache, checkpoint),
                    checkpoint,
                )?;
                buffer.merge_layered_scan(head, base)
            });
            return self.observe(result);
        }
        let result = cursor::scan(&ReadContext::new(&self.file, &self.page_cache, head), head);
        self.observe(result)
    }

    fn scan_range(
        &self,
        head: Head,
        start: &[u8],
        start_exclusive: bool,
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        if let Some(buffer) = &self.buffered_writes {
            if limit == 0 || end.is_some_and(|end| end <= start) {
                return Ok(Vec::new());
            }
            let result = buffer.stable_read(|| {
                let checkpoint = buffer.physical_checkpoint()?;
                if checkpoint.root().is_none() {
                    return buffer.merge_layered_range(
                        head,
                        Vec::new(),
                        start,
                        start_exclusive,
                        end,
                        limit,
                    );
                }
                let changes = buffer.layered_range_changes(head, start, start_exclusive, end)?;
                let base_limit = limit.checked_add(changes).ok_or(Error::FileTooLarge)?;
                let base = cursor::range(
                    &ReadContext::new(&self.file, &self.page_cache, checkpoint),
                    checkpoint,
                    start,
                    start_exclusive,
                    end,
                    base_limit,
                )?;
                buffer.merge_layered_range(head, base, start, start_exclusive, end, limit)
            });
            return self.observe(result);
        }
        let result = cursor::range(
            &ReadContext::new(&self.file, &self.page_cache, head),
            head,
            start,
            start_exclusive,
            end,
            limit,
        );
        self.observe(result)
    }

    fn verify(&self, head: Head) -> Result<VerificationReport> {
        if let Some(buffer) = &self.buffered_writes {
            let _guard = self.lock_commit()?;
            let checkpoint = buffer.physical_checkpoint()?;
            if buffer.requires_materialized_verification()? {
                let physical = tree::verify(
                    &UncachedReadContext::new(&self.file, checkpoint),
                    checkpoint,
                )?;
                let entries = self.scan(head)?;
                return self.observe(tree_verify::verify_materialized(head, &entries, physical));
            }
            let state = Self::buffer_state(buffer)?;
            let result =
                state.verification(&UncachedReadContext::new(&self.file, checkpoint), head);
            return self.observe(result);
        }
        let result = tree::verify(&UncachedReadContext::new(&self.file, head), head);
        self.observe(result)
    }

    fn page_cache_stats(&self) -> Result<PageCacheStats> {
        self.page_cache.stats()
    }

    fn checkpoint_stats(&self) -> Result<CheckpointStats> {
        Ok(self.metrics.checkpoint_stats())
    }

    fn backup(&self, target: &Path) -> Result<BackupReport> {
        self.backup_to(target)
    }

    fn compact_to(&self, target: &Path) -> Result<CompactionReport> {
        self.compact_to_path(target)
    }

    fn flush(&self, head: Head) -> Result<CommittedStorage> {
        if let Some(buffer) = &self.buffered_writes {
            return self.flush_buffered(buffer, head);
        }
        let _guard = self.lock_commit()?;
        let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
        self.reject_external_buffer()?;
        let file = open_database(&self.path)?;
        if !same_file_identity(&self.file, &file)? || load_head(&file)? != head {
            return Err(Error::WriteConflict);
        }
        Ok(CommittedStorage { store: None, head })
    }

    fn compact_active(&self, head: Head) -> Result<CompactedStorage> {
        self.compact_active_store(head)
    }

    fn refresh(&self) -> Result<RefreshedStorage> {
        if let Some(buffer) = &self.buffered_writes
            && let Some(head) = self.refresh_buffered_tail(buffer)?
        {
            return Ok(RefreshedStorage { store: None, head });
        }
        if self.buffered_writes.is_none() {
            let _guard = self.lock_commit()?;
            let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
            let file = open_database(&self.path)?;
            if same_file_identity(&self.file, &file)? && wal::file_len(&self.path)? == 0 {
                return Ok(RefreshedStorage {
                    store: None,
                    head: load_head(&file)?,
                });
            }
        }
        let capacity = self.page_cache.stats()?.capacity;
        let (store, head) = Self::open_with_metrics(
            &self.path,
            capacity,
            self.write_buffer_bytes,
            Arc::clone(&self.metrics),
            false,
        )?;
        Ok(RefreshedStorage {
            store: Some(Arc::new(store)),
            head,
        })
    }

    fn operational_stats(&self) -> Result<OperationalStats> {
        if let Some(buffer) = &self.buffered_writes {
            let (head, checkpoint, wal_bytes, main_file_bytes) = {
                let _guard = self.lock_commit()?;
                let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
                let state = Self::buffer_state(buffer)?;
                let checkpoint = buffer.physical_checkpoint()?;
                let file = open_database(&self.path)?;
                if !same_file_identity(&self.file, &file)?
                    || load_head(&file)? != checkpoint
                    || !self.buffer_file_matches(buffer, &state)?
                {
                    return Err(Error::WriteConflict);
                }
                (
                    state.head(),
                    checkpoint,
                    state.wal_bytes(),
                    self.file.metadata()?.len(),
                )
            };
            let verification = self.verify(head)?;
            let main_committed_bytes = checkpoint
                .last_page()
                .checked_add(1)
                .and_then(|pages| pages.checked_mul(PAGE_SIZE as u64))
                .ok_or(Error::FileTooLarge)?;
            let committed_bytes = main_committed_bytes
                .checked_add(wal_bytes)
                .ok_or(Error::FileTooLarge)?;
            let live_pages = verification
                .tree_pages
                .checked_add(verification.overflow_pages)
                .and_then(|pages| pages.checked_add(METADATA_SLOTS as u64))
                .ok_or(Error::FileTooLarge)?;
            let main_live_bytes = live_pages
                .checked_mul(PAGE_SIZE as u64)
                .ok_or(Error::FileTooLarge)?;
            let live_bytes = main_live_bytes
                .checked_add(wal_bytes)
                .ok_or(Error::FileTooLarge)?;
            let file_bytes = main_file_bytes
                .checked_add(wal_bytes)
                .ok_or(Error::FileTooLarge)?;
            let runtime = self.metrics.snapshot();
            return Ok(OperationalStats {
                snapshot: verification,
                file_bytes,
                committed_bytes,
                live_bytes,
                retired_bytes: main_committed_bytes.saturating_sub(main_live_bytes),
                uncommitted_tail_bytes: main_file_bytes.saturating_sub(main_committed_bytes),
                page_cache: self.page_cache_stats()?,
                commits: runtime.commits,
                syncs: runtime.syncs,
                wal_preflights: runtime.wal_preflights,
                wal_encodes: runtime.wal_encodes,
                wal_writes: runtime.wal_writes,
                wal_syncs: runtime.wal_syncs,
                wal_publications: runtime.wal_publications,
                checkpoints: runtime.checkpoints,
                checkpoint_starts: runtime.checkpoint_starts,
                checkpoints_in_progress: runtime.checkpoints_in_progress,
                checkpoint_bytes: runtime.checkpoint_bytes,
                writes: runtime.writes,
                corruption_detections: runtime.corruption_detections,
                checkpoint_failures: runtime.checkpoint_failures,
            });
        }
        let (source, head, file_bytes) = {
            let _guard = self.lock_commit()?;
            let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
            self.reject_external_buffer()?;
            let source = open_database(&self.path)?;
            let head = load_head(&source)?;
            let file_bytes = source.metadata()?.len();
            (source, head, file_bytes)
        };
        let verification =
            self.observe(tree::verify(&UncachedReadContext::new(&source, head), head))?;
        let committed_bytes = head
            .last_page()
            .checked_add(1)
            .and_then(|pages| pages.checked_mul(PAGE_SIZE as u64))
            .ok_or(Error::FileTooLarge)?;
        let live_pages = verification
            .tree_pages
            .checked_add(verification.overflow_pages)
            .and_then(|pages| pages.checked_add(METADATA_SLOTS as u64))
            .ok_or(Error::FileTooLarge)?;
        let live_bytes = live_pages
            .checked_mul(PAGE_SIZE as u64)
            .ok_or(Error::FileTooLarge)?;
        let runtime = self.metrics.snapshot();
        Ok(OperationalStats {
            snapshot: verification,
            file_bytes,
            committed_bytes,
            live_bytes,
            retired_bytes: committed_bytes.saturating_sub(live_bytes),
            uncommitted_tail_bytes: file_bytes.saturating_sub(committed_bytes),
            page_cache: self.page_cache_stats()?,
            commits: runtime.commits,
            syncs: runtime.syncs,
            wal_preflights: runtime.wal_preflights,
            wal_encodes: runtime.wal_encodes,
            wal_writes: runtime.wal_writes,
            wal_syncs: runtime.wal_syncs,
            wal_publications: runtime.wal_publications,
            checkpoints: runtime.checkpoints,
            checkpoint_starts: runtime.checkpoint_starts,
            checkpoints_in_progress: runtime.checkpoints_in_progress,
            checkpoint_bytes: runtime.checkpoint_bytes,
            writes: runtime.writes,
            corruption_detections: runtime.corruption_detections,
            checkpoint_failures: runtime.checkpoint_failures,
        })
    }

    fn commit(&self, base: Head, changes: ChangeSet) -> Result<CommittedStorage> {
        let commit_started = Instant::now();
        let _guard = self.lock_commit()?;
        let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
        if let Some(buffer) = &self.buffered_writes {
            return self.commit_buffered(buffer, base, changes, commit_started);
        }
        let file = open_database(&self.path)?;
        self.reject_external_buffer()?;
        if !same_file_identity(&self.file, &file)? || load_head(&file)? != base {
            return Err(Error::WriteConflict);
        }

        let transaction_id = base.transaction_id().next()?;
        let file_len = file.metadata()?.len();
        let first_new_page = next_page_after(file_len)?;
        let mut writer =
            CommitContext::new(&file, &self.page_cache, transaction_id, first_new_page);
        let (candidate_root, entry_count) = tree::apply(&mut writer, base, &changes)?;
        let root = writer.flush_reachable(candidate_root)?;
        let physical_data_page_bytes = writer
            .next_physical_page()
            .checked_sub(first_new_page)
            .and_then(|pages| pages.checked_mul(PAGE_SIZE as u64))
            .ok_or(Error::FileTooLarge)?;
        let page_write_bytes = writer.page_write_bytes();
        let data_page_bytes = page_write_bytes.total()?;
        if data_page_bytes != physical_data_page_bytes {
            return Err(Error::InternalInvariant("page write accounting mismatch"));
        }
        let logical_mutation_bytes = changes.logical_bytes()?;
        let data_sync_started = Instant::now();
        fault_injection::before("data_sync")?;
        file.sync_data()?;
        let data_sync = data_sync_started.elapsed();
        crash_injection::hit("data_sync");

        let last_page = if writer.next_physical_page() == first_new_page {
            base.last_page()
        } else {
            writer
                .next_physical_page()
                .checked_sub(1)
                .ok_or(Error::InternalInvariant("last page underflow"))?
        };
        let head = Head::new(transaction_id, root, last_page, entry_count);
        write_all_at(
            &file,
            &metadata::encode(head),
            metadata::slot_offset(transaction_id),
        )
        .map_err(Error::CommitOutcomeUnknown)?;
        crash_injection::hit("metadata_write");
        let metadata_sync_started = Instant::now();
        fault_injection::before("metadata_sync").map_err(Error::CommitOutcomeUnknown)?;
        file.sync_all().map_err(Error::CommitOutcomeUnknown)?;
        let sync = data_sync.saturating_add(metadata_sync_started.elapsed());
        crash_injection::hit("metadata_sync");
        self.metrics.record_commit(
            commit_started.elapsed(),
            sync,
            crate::WriteStats {
                logical_mutation_bytes,
                data_page_bytes,
                leaf_page_bytes: page_write_bytes.leaf,
                branch_page_bytes: page_write_bytes.branch,
                overflow_page_bytes: page_write_bytes.overflow,
                write_buffer_bytes: 0,
                metadata_bytes: PAGE_SIZE as u64,
            },
        );
        Ok(CommittedStorage { store: None, head })
    }

    fn commit_group(&self, base: Head, changes: Vec<ChangeSet>) -> Result<CommittedStorage> {
        if self.write_buffer_bytes == 0 {
            return Err(Error::InvalidInput(
                "group commit requires durable write buffering",
            ));
        }
        let Some(buffer) = &self.buffered_writes else {
            return Err(Error::InvalidInput(
                "group commit requires durable write buffering",
            ));
        };
        let commit_started = Instant::now();
        let _guard = self.lock_commit()?;
        let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
        buffered::group::commit(self, buffer, base, changes, commit_started)
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeMetrics, observe_corruption};
    use crate::Error;

    #[test]
    fn structural_corruption_variants_increment_the_counter() {
        let metrics = RuntimeMetrics::default();

        let corrupt = observe_corruption::<()>(&metrics, Err(Error::Corrupt("page".into())));
        assert!(matches!(corrupt, Err(Error::Corrupt(_))));
        let too_deep = observe_corruption::<()>(&metrics, Err(Error::TreeDepthExceeded));
        assert!(matches!(too_deep, Err(Error::TreeDepthExceeded)));
        let invalid = observe_corruption::<()>(&metrics, Err(Error::InvalidInput("input")));
        assert!(matches!(invalid, Err(Error::InvalidInput("input"))));

        assert_eq!(metrics.snapshot().corruption_detections, 2);
    }
}
