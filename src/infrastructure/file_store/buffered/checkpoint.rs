use std::fs::File;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::{BufferedWrites, FileStorageEngine, new_buffer_key_filter};
use crate::domain::Head;
use crate::infrastructure::file_context::{CommitContext, StreamingCommitContext, next_page_after};
use crate::infrastructure::membership_filter::{GrowingMembershipFilter, MembershipFilter};
use crate::infrastructure::metadata::{self, PAGE_SIZE};
use crate::infrastructure::metrics::{CheckpointActivity, RuntimeMetrics};
use crate::infrastructure::page_cache::PageCache;
use crate::infrastructure::path_lock::{AdvisoryFileLock, LockFile, MainFileLease};
use crate::infrastructure::positioned_io::write_all_at;
use crate::infrastructure::{bulk, crash_injection, fault_injection, tree, wal};
use crate::{Error, Result};

pub(super) struct CheckpointOutcome {
    pub(super) head: Head,
    pub(super) data_page_bytes: u64,
    pub(super) leaf_page_bytes: u64,
    pub(super) branch_page_bytes: u64,
    pub(super) overflow_page_bytes: u64,
    pub(super) metadata_bytes: u64,
    pub(super) sync: Duration,
    pub(super) elapsed: Duration,
    pub(super) membership_filter: Option<MembershipFilter>,
    pub(super) key_filter: GrowingMembershipFilter,
}

pub(super) struct PreparedCheckpoint {
    started: Instant,
    activity: CheckpointActivity,
    pub(super) head: Head,
    data_page_bytes: u64,
    leaf_page_bytes: u64,
    branch_page_bytes: u64,
    overflow_page_bytes: u64,
    membership_filter: Option<MembershipFilter>,
    key_filter: GrowingMembershipFilter,
    sync: Duration,
}

pub(super) struct PreparedWithLease {
    pub(super) lease: MainFileLease,
    pub(super) prepared: PreparedCheckpoint,
}

pub(super) struct CheckpointTask {
    worker: Mutex<WorkerState>,
}

impl Drop for CheckpointTask {
    fn drop(&mut self) {
        let worker = match self.worker.get_mut() {
            Ok(worker) => worker,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let WorkerState::Running(running) = std::mem::replace(worker, WorkerState::Consumed) {
            let _ = running.join();
        }
    }
}

pub(super) struct CheckpointWorkerResources {
    pub(super) file: File,
    pub(super) page_cache: Arc<PageCache>,
    pub(super) commit_lock: Arc<Mutex<()>>,
    pub(super) lock_file: LockFile,
    pub(super) metrics: Arc<RuntimeMetrics>,
}

enum WorkerState {
    Running(JoinHandle<Result<PreparedCheckpoint>>),
    Ready(Result<PreparedCheckpoint>),
    Consumed,
}

impl CheckpointTask {
    pub(super) fn spawn(
        buffer: Arc<BufferedWrites>,
        threshold: u64,
        lease: MainFileLease,
        resources: CheckpointWorkerResources,
    ) -> Result<Self> {
        let CheckpointWorkerResources {
            file,
            page_cache,
            commit_lock,
            lock_file,
            metrics,
        } = resources;
        let activity = metrics.checkpoint_activity();
        let worker = thread::Builder::new()
            .name("wsldb-checkpoint".into())
            .spawn(move || {
                let prepared = FileStorageEngine::prepare_checkpoint(
                    &buffer,
                    &file,
                    &page_cache,
                    threshold,
                    activity,
                    Some((&commit_lock, &lock_file)),
                );
                drop(lease);
                prepared
            })?;
        Ok(Self {
            worker: Mutex::new(WorkerState::Running(worker)),
        })
    }

    pub(super) fn wait_until_ready(&self) -> Result<()> {
        let mut worker = match self.worker.lock() {
            Ok(worker) => worker,
            Err(_) => return Err(Error::LockPoisoned("checkpoint worker")),
        };
        let WorkerState::Running(_) = &*worker else {
            return Ok(());
        };
        let running = match std::mem::replace(&mut *worker, WorkerState::Consumed) {
            WorkerState::Running(running) => running,
            _ => {
                return Err(Error::InternalInvariant(
                    "checkpoint worker state changed while locked",
                ));
            }
        };
        *worker = WorkerState::Ready(join_worker(running));
        Ok(())
    }

    pub(super) fn is_ready(&self) -> Result<bool> {
        let worker = self
            .worker
            .lock()
            .map_err(|_| Error::LockPoisoned("checkpoint worker"))?;
        Ok(match &*worker {
            WorkerState::Running(worker) => worker.is_finished(),
            WorkerState::Ready(_) => true,
            WorkerState::Consumed => false,
        })
    }

    pub(super) fn take_ready(&self) -> Option<Result<PreparedCheckpoint>> {
        let mut worker = match self.worker.lock() {
            Ok(worker) => worker,
            Err(_) => return Some(Err(Error::LockPoisoned("checkpoint worker"))),
        };
        if let WorkerState::Running(running) = &*worker
            && !running.is_finished()
        {
            return None;
        }
        let state = std::mem::replace(&mut *worker, WorkerState::Consumed);
        match state {
            WorkerState::Running(running) => Some(join_worker(running)),
            WorkerState::Ready(prepared) => Some(prepared),
            WorkerState::Consumed => Some(Err(Error::InternalInvariant(
                "checkpoint worker result was already consumed",
            ))),
        }
    }
}

fn join_worker(worker: JoinHandle<Result<PreparedCheckpoint>>) -> Result<PreparedCheckpoint> {
    worker
        .join()
        .map_err(|_| Error::InternalInvariant("checkpoint worker panicked"))?
}

impl FileStorageEngine {
    pub(super) fn checkpoint_buffer(
        &self,
        buffer: &BufferedWrites,
        file: &File,
        _lease: &MainFileLease,
    ) -> Result<CheckpointOutcome> {
        let activity = self.metrics.checkpoint_activity();
        let prepared = Self::prepare_checkpoint(
            buffer,
            file,
            &self.page_cache,
            self.write_buffer_bytes,
            activity,
            None,
        )?;
        buffer.mark_main_may_have_advanced();
        let outcome = Self::publish_prepared(file, prepared)?;
        self.metrics.record_checkpoint_completion(
            outcome.elapsed,
            outcome
                .data_page_bytes
                .saturating_add(outcome.metadata_bytes),
        );
        fault_injection::before("checkpoint_wal_clear")
            .map_err(Error::PublicationOutcomeUnknown)?;
        wal::truncate(&buffer.file, 0).map_err(super::super::publication_error)?;
        crash_injection::hit("checkpoint_wal_clear");
        Ok(outcome)
    }

    pub(super) fn prepare_checkpoint(
        buffer: &BufferedWrites,
        file: &File,
        page_cache: &PageCache,
        threshold: u64,
        activity: CheckpointActivity,
        sync_locks: Option<(&Mutex<()>, &LockFile)>,
    ) -> Result<PreparedCheckpoint> {
        let started = Instant::now();
        let key_filter = new_buffer_key_filter(threshold)?;
        let state = Self::buffer_state(buffer)?;
        let checkpoint = state.checkpoint();
        let logical_head = state.head();
        let can_stream = checkpoint.root().is_none() && state.latest_are_puts()?;

        let first_new_page = next_page_after(file.metadata()?.len())?;
        let (root, entry_count, next_physical_page, page_write_bytes, membership_filter) =
            if can_stream {
                let mut membership_filter =
                    MembershipFilter::for_entries(logical_head.entry_count())?;
                let mut writer = StreamingCommitContext::new(
                    file,
                    page_cache,
                    logical_head.transaction_id(),
                    first_new_page,
                );
                let mut loader = bulk::BulkLoader::new(&mut writer);
                state.visit_latest(|key, value| {
                    let value =
                        value.ok_or(Error::InternalInvariant("bulk put value is missing"))?;
                    membership_filter.insert(key);
                    loader.push(key, value)
                })?;
                let (root, entry_count) = loader.finish()?;
                writer.flush_pages()?;
                drop(state);
                (
                    root,
                    entry_count,
                    writer.next_physical_page(),
                    writer.page_write_bytes(),
                    Some(membership_filter),
                )
            } else {
                let changes = state.latest_changes()?;
                let mut membership_filter = buffer.membership_filter.clone();
                if let Some(filter) = &mut membership_filter {
                    for (key, value) in changes.sorted_iter() {
                        if value.is_some() {
                            filter.insert(key);
                        }
                    }
                }
                drop(state);
                let mut writer = CommitContext::new(
                    file,
                    page_cache,
                    logical_head.transaction_id(),
                    first_new_page,
                );
                let (candidate_root, entry_count) = tree::apply(&mut writer, checkpoint, &changes)?;
                let root = writer.flush_reachable(candidate_root)?;
                (
                    root,
                    entry_count,
                    writer.next_physical_page(),
                    writer.page_write_bytes(),
                    membership_filter,
                )
            };
        if entry_count != logical_head.entry_count() {
            return Err(Error::Corrupt(
                "write-buffer checkpoint entry count is invalid".into(),
            ));
        }
        let data_page_bytes = page_write_bytes.total()?;
        let physical_data_page_bytes = next_physical_page
            .checked_sub(first_new_page)
            .and_then(|pages| pages.checked_mul(PAGE_SIZE as u64))
            .ok_or(Error::FileTooLarge)?;
        if data_page_bytes != physical_data_page_bytes {
            return Err(Error::InternalInvariant(
                "checkpoint page write accounting mismatch",
            ));
        }
        let sync_started = Instant::now();
        let commit_guard = match sync_locks {
            Some((commit_lock, _)) => Some(
                commit_lock
                    .lock()
                    .map_err(|_| Error::LockPoisoned("file commit"))?,
            ),
            None => None,
        };
        let file_guard = match sync_locks {
            Some((_, lock_file)) => Some(AdvisoryFileLock::acquire(lock_file)?),
            None => None,
        };
        fault_injection::before("checkpoint_data_sync")?;
        file.sync_data()?;
        let sync = sync_started.elapsed();
        drop(file_guard);
        drop(commit_guard);
        crash_injection::hit("checkpoint_data_sync");
        let last_page = if next_physical_page == first_new_page {
            checkpoint.last_page()
        } else {
            next_physical_page
                .checked_sub(1)
                .ok_or(Error::InternalInvariant("checkpoint last page underflow"))?
        };
        let head = Head::new(logical_head.transaction_id(), root, last_page, entry_count);
        Ok(PreparedCheckpoint {
            started,
            activity,
            head,
            data_page_bytes,
            leaf_page_bytes: page_write_bytes.leaf,
            branch_page_bytes: page_write_bytes.branch,
            overflow_page_bytes: page_write_bytes.overflow,
            membership_filter,
            key_filter,
            sync,
        })
    }

    pub(super) fn publish_prepared(
        file: &File,
        prepared: PreparedCheckpoint,
    ) -> Result<CheckpointOutcome> {
        let PreparedCheckpoint {
            started,
            activity: _activity,
            head,
            data_page_bytes,
            leaf_page_bytes,
            branch_page_bytes,
            overflow_page_bytes,
            membership_filter,
            key_filter,
            sync,
        } = prepared;
        let metadata_started = Instant::now();
        fault_injection::before("checkpoint_metadata_write")?;
        write_all_at(
            file,
            &metadata::encode(head),
            metadata::slot_offset(head.transaction_id()),
        )?;
        crash_injection::hit("checkpoint_metadata_write");
        fault_injection::before("checkpoint_metadata_sync")?;
        file.sync_all()?;
        crash_injection::hit("checkpoint_metadata_sync");
        Ok(CheckpointOutcome {
            head,
            data_page_bytes,
            leaf_page_bytes,
            branch_page_bytes,
            overflow_page_bytes,
            metadata_bytes: PAGE_SIZE as u64,
            sync: sync.saturating_add(metadata_started.elapsed()),
            elapsed: started.elapsed(),
            membership_filter,
            key_filter,
        })
    }
}
