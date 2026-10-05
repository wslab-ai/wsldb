use std::fs::File;
use std::path::Path;
use std::sync::{Arc, RwLock, RwLockReadGuard};
use std::time::{Duration, Instant};

use super::FileStorageEngine;
use crate::application::{CommittedStorage, StorageEngine};
use crate::domain::Head;
use crate::infrastructure::file_context::load_head;
use crate::infrastructure::membership_filter::{GrowingMembershipFilter, MembershipFilter};
use crate::infrastructure::path_lock::{
    AdvisoryFileLock, MainFileLease, current_file_identity_matches, open_database,
    same_file_identity,
};
use crate::infrastructure::write_buffer::{PointIndexSlot, WriteBuffer};
use crate::infrastructure::{crash_injection, fault_injection, wal};
use crate::{Error, Result};

mod checkpoint;
mod commit;
pub(super) mod group;
mod recovery;
mod state;

use checkpoint::{CheckpointTask, CheckpointWorkerResources, PreparedWithLease};
pub(super) use state::BufferedWrites;
use state::{
    BufferedBase, DurableBufferCommit, PendingPublication, ReplacementResources, SnapshotOutcome,
    new_buffer_key_filter,
};

impl FileStorageEngine {
    const fn buffer_checkpoint_bytes(&self) -> u64 {
        if self.write_buffer_bytes == 0 {
            super::RECOVERED_WRITE_BUFFER_BYTES
        } else {
            self.write_buffer_bytes
        }
    }

    pub(super) fn open_buffer(
        path: &Path,
        checkpoint: Head,
        threshold: u64,
    ) -> Result<(Option<BufferedWrites>, Head)> {
        recovery::open(path, checkpoint, threshold)
    }

    pub(super) fn buffer_state(
        buffer: &BufferedWrites,
    ) -> Result<RwLockReadGuard<'_, WriteBuffer>> {
        buffer
            .state
            .read()
            .map_err(|_| Error::LockPoisoned("write buffer"))
    }

    pub(super) fn buffer_file_matches(
        &self,
        buffer: &BufferedWrites,
        state: &WriteBuffer,
    ) -> Result<bool> {
        Ok(wal::is_current(&buffer.file, &self.path)?
            && buffer.file.metadata()?.len() == state.wal_bytes())
    }

    pub(super) fn open_main_for_buffered_commit(
        &self,
        buffer: &BufferedWrites,
        state: &WriteBuffer,
        physical_checkpoint: Head,
    ) -> Result<Option<File>> {
        if state.wal_bytes() != 0 && !buffer.main_requires_validation() {
            if !current_file_identity_matches(&self.path, &self.file)? {
                return Err(Error::WriteConflict);
            }
            return Ok(None);
        }
        let file = open_database(&self.path)?;
        if !same_file_identity(&self.file, &file)? || load_head(&file)? != physical_checkpoint {
            return Err(Error::WriteConflict);
        }
        buffer.mark_main_validated();
        Ok(Some(file))
    }

    pub(super) fn refresh_buffered_tail(&self, buffer: &BufferedWrites) -> Result<Option<Head>> {
        let _guard = self.lock_commit()?;
        let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
        let file = open_database(&self.path)?;
        self.refresh_buffered_tail_locked(buffer, &file)
    }

    pub(super) fn refresh_buffered_tail_locked(
        &self,
        buffer: &BufferedWrites,
        file: &File,
    ) -> Result<Option<Head>> {
        if buffer.requires_materialized_verification()? {
            return Ok(None);
        }
        let (checkpoint, base, start) = {
            let state = Self::buffer_state(buffer)?;
            (state.checkpoint(), state.head(), state.wal_bytes())
        };
        if load_head(file)? != checkpoint || !same_file_identity(&self.file, file)? {
            return Ok(None);
        }
        if !wal::is_current(&buffer.file, &self.path)? {
            if start != 0 && wal::file_len(&self.path)? == 0 {
                return Err(Error::WriteConflict);
            }
            return Ok(None);
        }

        let file_len = buffer.file.metadata()?.len();
        if file_len < start {
            return Ok(None);
        }
        if file_len == start {
            return Ok(Some(base));
        }

        let mut records = Vec::new();
        let scan = wal::replay_after(&buffer.file, start, base.transaction_id(), |record| {
            records.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
            records.push(record);
            Ok(())
        })?;
        if scan.has_incomplete_tail {
            wal::truncate(&buffer.file, scan.valid_bytes)?;
        }
        let mut state = buffer
            .state
            .write()
            .map_err(|_| Error::LockPoisoned("write buffer"))?;
        if state.checkpoint() != checkpoint || state.head() != base || state.wal_bytes() != start {
            return Err(Error::InternalInvariant(
                "write buffer changed during serialized refresh",
            ));
        }
        let point_index = buffer.point_index.get();
        if let Some(point_index) = point_index {
            for record in &records {
                point_index.preflight(&record.changes, &state)?;
            }
        }

        let mutations = records.iter().try_fold(0_usize, |total, record| {
            total
                .checked_add(record.changes.len())
                .ok_or(Error::FileTooLarge)
        })?;
        let mut point_updates = Vec::new();
        if point_index.is_some() {
            point_updates
                .try_reserve_exact(mutations)
                .map_err(|_| Error::FileTooLarge)?;
        }
        state.prepare_apply(records.iter().map(|record| &record.changes))?;
        for record in records {
            state.apply_durable(record, point_index.map(|_| &mut point_updates));
        }
        state.set_wal_bytes(scan.valid_bytes);
        let head = state.head();
        if let Some(point_index) = point_index {
            point_index.apply_durable(point_updates);
        }
        drop(state);
        Ok(Some(head))
    }

    fn prepare_replacement(&self, file: &File, wal_file: &File) -> Result<ReplacementResources> {
        Ok(ReplacementResources {
            file: file.try_clone()?,
            lock_file: self.lock_file.try_clone()?,
            wal_file: wal_file.try_clone()?,
            page_cache: Arc::clone(&self.page_cache),
        })
    }

    fn replacement_after_checkpoint(
        &self,
        resources: ReplacementResources,
        head: Head,
        membership_filter: Option<MembershipFilter>,
        key_filter: GrowingMembershipFilter,
    ) -> Arc<dyn StorageEngine> {
        if self.write_buffer_bytes == 0 {
            return Arc::new(Self {
                file: resources.file,
                path: self.path.clone(),
                lock_file: resources.lock_file,
                commit_lock: Arc::clone(&self.commit_lock),
                page_cache: resources.page_cache,
                write_buffer_bytes: 0,
                buffered_writes: None,
                recovery_view: None,
                metrics: Arc::clone(&self.metrics),
            });
        }
        let state = WriteBuffer::empty_with_membership_filter(head, key_filter);
        Arc::new(Self {
            file: resources.file,
            path: self.path.clone(),
            lock_file: resources.lock_file,
            commit_lock: Arc::clone(&self.commit_lock),
            page_cache: resources.page_cache,
            write_buffer_bytes: self.write_buffer_bytes,
            buffered_writes: Some(Arc::new(BufferedWrites {
                file: resources.wal_file,
                state: RwLock::new(state),
                checkpoint: head,
                point_index: Arc::new(PointIndexSlot::new()),
                membership_filter,
                physical_checkpoint: RwLock::new(head),
                base: RwLock::new(None),
                transition_epoch: std::sync::atomic::AtomicU64::new(0),
                main_may_have_advanced: std::sync::atomic::AtomicBool::new(false),
                checkpoint_task: None,
                wal_start: 0,
            })),
            recovery_view: None,
            metrics: Arc::clone(&self.metrics),
        })
    }

    fn replacement_after_snapshot(
        &self,
        resources: ReplacementResources,
        buffer: BufferedWrites,
    ) -> Arc<dyn StorageEngine> {
        Arc::new(Self {
            file: resources.file,
            path: self.path.clone(),
            lock_file: resources.lock_file,
            commit_lock: Arc::clone(&self.commit_lock),
            page_cache: resources.page_cache,
            write_buffer_bytes: self.write_buffer_bytes,
            buffered_writes: Some(Arc::new(buffer)),
            recovery_view: None,
            metrics: Arc::clone(&self.metrics),
        })
    }

    fn replacement_with_pending_checkpoint(
        &self,
        buffer: &BufferedWrites,
        file: &File,
        logical_head: Head,
        wal_bytes: u64,
        lease: MainFileLease,
    ) -> Result<Arc<dyn StorageEngine>> {
        let frozen = self
            .buffered_writes
            .as_ref()
            .filter(|current| std::ptr::eq(current.as_ref(), buffer))
            .cloned()
            .ok_or(Error::InternalInvariant(
                "checkpoint buffer does not belong to its storage engine",
            ))?;
        if buffer.requires_materialized_verification()? || buffer.checkpoint_task.is_some() {
            return Err(Error::InternalInvariant(
                "nested checkpoint generation is unsupported",
            ));
        }
        let resources = self.prepare_replacement(file, &buffer.file)?;
        let threshold = self.buffer_checkpoint_bytes();
        let key_filter = new_buffer_key_filter(threshold)?;
        let task = CheckpointTask::spawn(
            Arc::clone(&frozen),
            threshold,
            lease,
            CheckpointWorkerResources {
                file: file.try_clone()?,
                page_cache: Arc::clone(&self.page_cache),
                commit_lock: Arc::clone(&self.commit_lock),
                lock_file: self.lock_file.try_clone()?,
                metrics: Arc::clone(&self.metrics),
            },
        )?;
        let mut state = WriteBuffer::empty_with_membership_filter(logical_head, key_filter);
        state.set_wal_bytes(wal_bytes);
        Ok(Arc::new(Self {
            file: resources.file,
            path: self.path.clone(),
            lock_file: resources.lock_file,
            commit_lock: Arc::clone(&self.commit_lock),
            page_cache: resources.page_cache,
            write_buffer_bytes: self.write_buffer_bytes,
            buffered_writes: Some(Arc::new(BufferedWrites {
                file: resources.wal_file,
                state: RwLock::new(state),
                checkpoint: logical_head,
                point_index: Arc::new(PointIndexSlot::new()),
                membership_filter: None,
                physical_checkpoint: RwLock::new(buffer.physical_checkpoint()?),
                base: RwLock::new(Some(BufferedBase {
                    buffer: frozen,
                    head: logical_head,
                })),
                transition_epoch: std::sync::atomic::AtomicU64::new(0),
                main_may_have_advanced: std::sync::atomic::AtomicBool::new(false),
                checkpoint_task: Some(task),
                wal_start: wal_bytes,
            })),
            recovery_view: None,
            metrics: Arc::clone(&self.metrics),
        }))
    }

    fn snapshot_buffer(
        &self,
        buffer: &BufferedWrites,
        checkpoint: Head,
        membership_filter: Option<MembershipFilter>,
    ) -> Result<Option<SnapshotOutcome>> {
        let state = Self::buffer_state(buffer)?;
        let source_head = state.head();
        let key_filter = new_buffer_key_filter(self.write_buffer_bytes)?;
        let mut snapshot_state = state.snapshot_rebased(checkpoint, key_filter, 0)?;

        let Some(replacement) = wal::replace_with_snapshot_streamed(
            &self.path,
            checkpoint.transaction_id(),
            source_head.transaction_id(),
            source_head.entry_count(),
            &state,
        )?
        else {
            return Ok(None);
        };
        snapshot_state.set_wal_bytes(replacement.bytes);
        let snapshot_head = snapshot_state.head();
        if snapshot_head.transaction_id() != source_head.transaction_id()
            || snapshot_head.entry_count() != source_head.entry_count()
        {
            return Err(Error::InternalInvariant(
                "write-buffer snapshot changed the logical head",
            ));
        }
        Ok(Some(SnapshotOutcome {
            buffer: BufferedWrites {
                file: replacement.file,
                state: RwLock::new(snapshot_state),
                checkpoint,
                point_index: Arc::clone(&buffer.point_index),
                membership_filter,
                physical_checkpoint: RwLock::new(checkpoint),
                base: RwLock::new(None),
                transition_epoch: std::sync::atomic::AtomicU64::new(0),
                main_may_have_advanced: std::sync::atomic::AtomicBool::new(false),
                checkpoint_task: None,
                wal_start: 0,
            },
            head: snapshot_head,
            bytes: replacement.bytes,
            sync: replacement.sync,
        }))
    }

    fn publish_pending_checkpoint(
        &self,
        buffer: &BufferedWrites,
        file: &File,
        completion: PreparedWithLease,
    ) -> Result<PendingPublication> {
        let PreparedWithLease {
            lease: _lease,
            prepared,
        } = completion;
        let state = Self::buffer_state(buffer)?;
        let logical_head = state.head();
        let frozen_head = state.checkpoint();
        if prepared.head.transaction_id() != frozen_head.transaction_id()
            || prepared.head.entry_count() != frozen_head.entry_count()
            || !current_file_identity_matches(&self.path, &self.file)?
            || !same_file_identity(&self.file, file)?
            || load_head(file)? != buffer.physical_checkpoint()?
            || !self.buffer_file_matches(buffer, &state)?
        {
            return Err(Error::WriteConflict);
        }
        drop(state);

        buffer.mark_main_may_have_advanced();
        let checkpoint = Self::publish_prepared(file, prepared)?;
        self.metrics.record_checkpoint_completion(
            checkpoint.elapsed,
            checkpoint
                .data_page_bytes
                .saturating_add(checkpoint.metadata_bytes),
        );
        if logical_head == frozen_head {
            let resources = self.prepare_replacement(file, &buffer.file)?;
            fault_injection::before("checkpoint_wal_clear")
                .map_err(Error::PublicationOutcomeUnknown)?;
            wal::truncate(&buffer.file, 0).map_err(super::publication_error)?;
            crash_injection::hit("checkpoint_wal_clear");
            buffer.promote_physical_checkpoint(checkpoint.head)?;
            let store = self.replacement_after_checkpoint(
                resources,
                checkpoint.head,
                checkpoint.membership_filter.clone(),
                checkpoint.key_filter.clone(),
            );
            return Ok(PendingPublication {
                store,
                head: checkpoint.head,
                checkpoint,
                suffix_bytes: 0,
                suffix_sync: Duration::ZERO,
            });
        }

        buffer.promote_physical_checkpoint(checkpoint.head)?;
        let snapshot = self
            .snapshot_buffer(
                buffer,
                checkpoint.head,
                checkpoint.membership_filter.clone(),
            )?
            .ok_or(Error::InvalidInput(
                "checkpoint suffix replacement is unavailable",
            ))?;
        let resources = self.prepare_replacement(file, &snapshot.buffer.file)?;
        let store = self.replacement_after_snapshot(resources, snapshot.buffer);
        Ok(PendingPublication {
            store,
            head: snapshot.head,
            checkpoint,
            suffix_bytes: snapshot.bytes,
            suffix_sync: snapshot.sync,
        })
    }

    pub(super) fn flush_buffered(
        &self,
        buffer: &BufferedWrites,
        head: Head,
    ) -> Result<CommittedStorage> {
        if let Some(task) = &buffer.checkpoint_task {
            task.wait_until_ready()?;
            let main_file_lease = MainFileLease::acquire(&self.path)?;
            let _guard = self.lock_commit()?;
            let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
            let file = open_database(&self.path)?;
            if Self::buffer_state(buffer)?.head() != head {
                return Err(Error::WriteConflict);
            }
            let prepared = task.take_ready().ok_or(Error::InternalInvariant(
                "checkpoint worker was not ready after joining",
            ))??;
            let completion = PreparedWithLease {
                lease: main_file_lease,
                prepared,
            };
            let publication = self.publish_pending_checkpoint(buffer, &file, completion)?;
            self.metrics.record_maintenance(
                publication
                    .checkpoint
                    .sync
                    .saturating_add(publication.suffix_sync),
                crate::WriteStats {
                    logical_mutation_bytes: 0,
                    data_page_bytes: publication.checkpoint.data_page_bytes,
                    leaf_page_bytes: publication.checkpoint.leaf_page_bytes,
                    branch_page_bytes: publication.checkpoint.branch_page_bytes,
                    overflow_page_bytes: publication.checkpoint.overflow_page_bytes,
                    write_buffer_bytes: publication.suffix_bytes,
                    metadata_bytes: publication.checkpoint.metadata_bytes,
                },
            );
            return Ok(CommittedStorage {
                store: Some(publication.store),
                head: publication.head,
            });
        }
        let main_file_lease = MainFileLease::acquire(&self.path)?;
        let _guard = self.lock_commit()?;
        let _file_lock = AdvisoryFileLock::acquire(&self.lock_file)?;
        let file = open_database(&self.path)?;
        let state = Self::buffer_state(buffer)?;
        if state.head() != head
            || !same_file_identity(&self.file, &file)?
            || load_head(&file)? != buffer.physical_checkpoint()?
            || !self.buffer_file_matches(buffer, &state)?
        {
            return Err(Error::WriteConflict);
        }
        if state.wal_bytes() == 0 {
            return Ok(CommittedStorage { store: None, head });
        }
        if state.is_consolidated() && self.write_buffer_bytes != 0 {
            return Ok(CommittedStorage { store: None, head });
        }
        drop(state);

        let snapshot = if self.write_buffer_bytes == 0 {
            None
        } else {
            match self.snapshot_buffer(
                buffer,
                buffer.physical_checkpoint()?,
                buffer.membership_filter.clone(),
            ) {
                Ok(snapshot) => snapshot,
                Err(Error::FileTooLarge) => None,
                Err(error) => return Err(error),
            }
        };
        if let Some(snapshot) = snapshot {
            let resources = self.prepare_replacement(&file, &snapshot.buffer.file)?;
            self.metrics.record_maintenance(
                snapshot.sync,
                crate::WriteStats {
                    logical_mutation_bytes: 0,
                    data_page_bytes: 0,
                    leaf_page_bytes: 0,
                    branch_page_bytes: 0,
                    overflow_page_bytes: 0,
                    write_buffer_bytes: snapshot.bytes,
                    metadata_bytes: 0,
                },
            );
            return Ok(CommittedStorage {
                store: Some(self.replacement_after_snapshot(resources, snapshot.buffer)),
                head: snapshot.head,
            });
        }

        let resources = self.prepare_replacement(&file, &buffer.file)?;
        let checkpoint = self.checkpoint_buffer(buffer, &file, &main_file_lease)?;
        self.metrics.record_maintenance(
            checkpoint.sync,
            crate::WriteStats {
                logical_mutation_bytes: 0,
                data_page_bytes: checkpoint.data_page_bytes,
                leaf_page_bytes: checkpoint.leaf_page_bytes,
                branch_page_bytes: checkpoint.branch_page_bytes,
                overflow_page_bytes: checkpoint.overflow_page_bytes,
                write_buffer_bytes: 0,
                metadata_bytes: checkpoint.metadata_bytes,
            },
        );
        Ok(CommittedStorage {
            store: Some(self.replacement_after_checkpoint(
                resources,
                checkpoint.head,
                checkpoint.membership_filter,
                checkpoint.key_filter,
            )),
            head: checkpoint.head,
        })
    }

    fn finish_buffered_commit(
        &self,
        buffer: &BufferedWrites,
        file: &File,
        outcome: DurableBufferCommit,
        commit_started: Instant,
    ) -> CommittedStorage {
        let DurableBufferCommit {
            head: logical_head,
            wal_bytes: new_wal_bytes,
            mut writes,
            mut sync,
            preflight,
            publication_started,
        } = outcome;
        let mut committed = CommittedStorage {
            store: None,
            head: logical_head,
        };
        let generation_bytes = buffer.wal_generation_bytes(new_wal_bytes);
        let checkpoint_bytes = self.buffer_checkpoint_bytes();
        if let Some(task) = &buffer.checkpoint_task {
            let ready = if generation_bytes < checkpoint_bytes {
                false
            } else {
                match task.is_ready() {
                    Ok(ready) => ready,
                    Err(_) => {
                        self.metrics.record_checkpoint_failure();
                        false
                    }
                }
            };
            if ready {
                let lease = MainFileLease::try_acquire(&self.path);
                let completion = match lease {
                    Ok(Some(lease)) => task.take_ready().map(|prepared| {
                        prepared.map(|prepared| PreparedWithLease { lease, prepared })
                    }),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                };
                let Some(completion) = completion else {
                    self.record_buffered_commit(
                        commit_started,
                        sync,
                        writes,
                        preflight,
                        publication_started,
                    );
                    return committed;
                };
                let recovery = self.prepare_replacement(file, &buffer.file);
                match completion.and_then(|completion| {
                    self.publish_pending_checkpoint(buffer, file, completion)
                }) {
                    Ok(publication) => {
                        writes.data_page_bytes = publication.checkpoint.data_page_bytes;
                        writes.leaf_page_bytes = publication.checkpoint.leaf_page_bytes;
                        writes.branch_page_bytes = publication.checkpoint.branch_page_bytes;
                        writes.overflow_page_bytes = publication.checkpoint.overflow_page_bytes;
                        writes.metadata_bytes = publication.checkpoint.metadata_bytes;
                        writes.write_buffer_bytes = writes
                            .write_buffer_bytes
                            .saturating_add(publication.suffix_bytes);
                        sync = sync
                            .saturating_add(publication.checkpoint.sync)
                            .saturating_add(publication.suffix_sync);
                        committed.store = Some(publication.store);
                        committed.head = publication.head;
                    }
                    Err(_) => {
                        self.metrics.record_checkpoint_failure();
                        if let Ok(resources) = recovery
                            && let Ok((store, head)) =
                                self.replacement_after_failed_checkpoint(resources)
                        {
                            committed.store = Some(store);
                            committed.head = head;
                        }
                    }
                }
            }
        } else if generation_bytes >= checkpoint_bytes {
            match MainFileLease::try_acquire(&self.path) {
                Ok(Some(main_file_lease)) => match self.replacement_with_pending_checkpoint(
                    buffer,
                    file,
                    logical_head,
                    new_wal_bytes,
                    main_file_lease,
                ) {
                    Ok(store) => committed.store = Some(store),
                    Err(_) => self.metrics.record_checkpoint_failure(),
                },
                Ok(None) => {}
                Err(_) => self.metrics.record_checkpoint_failure(),
            }
        }
        self.record_buffered_commit(commit_started, sync, writes, preflight, publication_started);
        committed
    }

    fn record_buffered_commit(
        &self,
        commit_started: Instant,
        sync: Duration,
        writes: crate::WriteStats,
        preflight: Duration,
        publication_started: Instant,
    ) {
        self.metrics
            .record_wal_commit_edges(preflight, publication_started.elapsed());
        self.metrics
            .record_commit(commit_started.elapsed(), sync, writes);
    }
}
