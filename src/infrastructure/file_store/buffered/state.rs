use std::fs::File;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::application::StorageEngine;
use crate::domain::{ChangeSet, Head};
use crate::infrastructure::membership_filter::{GrowingMembershipFilter, MembershipFilter};
use crate::infrastructure::page_cache::PageCache;
use crate::infrastructure::path_lock::LockFile;
use crate::infrastructure::write_buffer::{PointIndex, PointIndexSlot, WriteBuffer};
use crate::{Error, Result};

const FILTER_EXPECTED_ENCODED_BYTES_PER_KEY: u64 = 64;

pub(super) fn new_buffer_key_filter(threshold: u64) -> Result<GrowingMembershipFilter> {
    let bytes = if threshold == 0 {
        super::super::RECOVERED_WRITE_BUFFER_BYTES
    } else {
        threshold
    };
    GrowingMembershipFilter::new(bytes.div_ceil(FILTER_EXPECTED_ENCODED_BYTES_PER_KEY))
}

pub(in crate::infrastructure::file_store) struct BufferedWrites {
    pub(in crate::infrastructure::file_store) file: File,
    pub(super) state: RwLock<WriteBuffer>,
    pub(super) checkpoint: Head,
    pub(super) point_index: Arc<PointIndexSlot>,
    pub(super) membership_filter: Option<MembershipFilter>,
    pub(super) physical_checkpoint: RwLock<Head>,
    pub(super) base: RwLock<Option<BufferedBase>>,
    pub(super) transition_epoch: AtomicU64,
    pub(super) main_may_have_advanced: AtomicBool,
    pub(super) checkpoint_task: Option<super::checkpoint::CheckpointTask>,
    pub(super) wal_start: u64,
}

pub(super) struct BufferedBase {
    pub(super) buffer: Arc<BufferedWrites>,
    pub(super) head: Head,
}

pub(super) struct SnapshotOutcome {
    pub(super) buffer: BufferedWrites,
    pub(super) head: Head,
    pub(super) bytes: u64,
    pub(super) sync: Duration,
}

pub(super) struct DurableBufferCommit {
    pub(super) head: Head,
    pub(super) wal_bytes: u64,
    pub(super) writes: crate::WriteStats,
    pub(super) sync: Duration,
    pub(super) preflight: Duration,
    pub(super) publication_started: Instant,
}

pub(super) struct PendingPublication {
    pub(super) store: Arc<dyn StorageEngine>,
    pub(super) head: Head,
    pub(super) checkpoint: super::checkpoint::CheckpointOutcome,
    pub(super) suffix_bytes: u64,
    pub(super) suffix_sync: Duration,
}

pub(super) struct ReplacementResources {
    pub(super) file: File,
    pub(super) lock_file: LockFile,
    pub(super) wal_file: File,
    pub(super) page_cache: Arc<PageCache>,
}

impl BufferedWrites {
    pub(super) fn mark_main_may_have_advanced(&self) {
        self.main_may_have_advanced.store(true, Ordering::Release);
    }

    pub(super) fn main_requires_validation(&self) -> bool {
        self.main_may_have_advanced.load(Ordering::Acquire)
    }

    pub(super) fn mark_main_validated(&self) {
        self.main_may_have_advanced.store(false, Ordering::Release);
    }

    pub(in crate::infrastructure::file_store) fn stable_read<T>(
        &self,
        mut read: impl FnMut() -> Result<T>,
    ) -> Result<T> {
        loop {
            let epoch = self.transition_epoch.load(Ordering::Acquire);
            if epoch % 2 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let result = read();
            if self.transition_epoch.load(Ordering::Acquire) == epoch {
                return result;
            }
        }
    }

    pub(super) fn visible_value(
        &self,
        snapshot: Head,
        key: &[u8],
    ) -> Result<Option<Option<Vec<u8>>>> {
        if snapshot.transaction_id() < self.checkpoint.transaction_id()
            || snapshot.root() != self.checkpoint.root()
            || snapshot.last_page() != self.checkpoint.last_page()
        {
            return Err(Error::InternalInvariant(
                "snapshot does not belong to this write buffer",
            ));
        }
        if let Some(point_index) = self.point_index.get() {
            return point_index.visible_value(snapshot.transaction_id(), key);
        }
        let point_index = self.point_index.get_or_initialize(|| {
            let state = self
                .state
                .read()
                .map_err(|_| Error::LockPoisoned("write buffer"))?;
            PointIndex::from_buffer(&state)
        })?;
        point_index.visible_value(snapshot.transaction_id(), key)
    }

    fn frozen_base(&self) -> Result<Option<(Arc<BufferedWrites>, Head)>> {
        Ok(self
            .base
            .read()
            .map_err(|_| Error::LockPoisoned("checkpoint base"))?
            .as_ref()
            .map(|base| (Arc::clone(&base.buffer), base.head)))
    }

    pub(super) fn checkpoint_may_contain(&self, key: &[u8]) -> Result<bool> {
        match self.frozen_base()? {
            Some((buffer, _)) => buffer.checkpoint_may_contain(key),
            None => Ok(self
                .membership_filter
                .as_ref()
                .is_none_or(|filter| filter.may_contain(key))),
        }
    }

    pub(in crate::infrastructure::file_store) fn physical_checkpoint(&self) -> Result<Head> {
        Ok(self
            .physical_checkpoint
            .read()
            .map_err(|_| Error::LockPoisoned("physical checkpoint"))?
            .to_owned())
    }

    pub(super) fn promote_physical_checkpoint(&self, checkpoint: Head) -> Result<()> {
        let epoch = self.transition_epoch.fetch_add(1, Ordering::AcqRel);
        if epoch % 2 != 0 {
            self.transition_epoch.fetch_add(1, Ordering::Release);
            return Err(Error::InternalInvariant(
                "checkpoint transition is already active",
            ));
        }
        let result = (|| {
            *self
                .physical_checkpoint
                .write()
                .map_err(|_| Error::LockPoisoned("physical checkpoint"))? = checkpoint;
            self.base
                .write()
                .map_err(|_| Error::LockPoisoned("checkpoint base"))?
                .take();
            Ok(())
        })();
        self.transition_epoch.fetch_add(1, Ordering::Release);
        result
    }

    pub(in crate::infrastructure::file_store) fn requires_materialized_verification(
        &self,
    ) -> Result<bool> {
        if self.frozen_base()?.is_some() {
            return Ok(true);
        }
        let physical = self.physical_checkpoint()?;
        let logical = self
            .state
            .read()
            .map_err(|_| Error::LockPoisoned("write buffer"))?
            .checkpoint();
        Ok(physical != logical)
    }

    pub(in crate::infrastructure::file_store) fn layered_visible_value(
        &self,
        snapshot: Head,
        key: &[u8],
    ) -> Result<Option<Option<Vec<u8>>>> {
        if let Some(value) = self.visible_value(snapshot, key)? {
            return Ok(Some(value));
        }
        let Some((base, head)) = self.frozen_base()? else {
            return Ok(None);
        };
        base.layered_visible_value(head, key)
    }

    pub(super) fn base_visible_presence(&self, key: &[u8]) -> Result<Option<bool>> {
        let Some((base, head)) = self.frozen_base()? else {
            return Ok(None);
        };
        let state = base
            .state
            .read()
            .map_err(|_| Error::LockPoisoned("write buffer"))?;
        if let Some(present) = state.visible_presence(head, key)? {
            return Ok(Some(present));
        }
        drop(state);
        base.base_visible_presence(key)
    }

    pub(in crate::infrastructure::file_store) fn merge_layered_scan(
        &self,
        snapshot: Head,
        physical: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let base = match self.frozen_base()? {
            Some((base, head)) => base.merge_layered_scan(head, physical)?,
            None => physical,
        };
        self.state
            .read()
            .map_err(|_| Error::LockPoisoned("write buffer"))?
            .merge_scan(snapshot, base)
    }

    pub(in crate::infrastructure::file_store) fn layered_latest_changes(
        &self,
        snapshot: Head,
    ) -> Result<ChangeSet> {
        let mut changes = match self.frozen_base()? {
            Some((base, head)) => base.layered_latest_changes(head)?,
            None => ChangeSet::new(),
        };
        let state = self
            .state
            .read()
            .map_err(|_| Error::LockPoisoned("write buffer"))?;
        if state.head() != snapshot {
            return Err(Error::InternalInvariant(
                "buffered maintenance snapshot changed",
            ));
        }
        for (key, value) in state.latest_changes()?.into_sorted_iter() {
            changes.insert_shared(key, value);
        }
        Ok(changes)
    }

    pub(in crate::infrastructure::file_store) fn layered_range_changes(
        &self,
        snapshot: Head,
        start: &[u8],
        start_exclusive: bool,
        end: Option<&[u8]>,
    ) -> Result<usize> {
        let current = self
            .state
            .read()
            .map_err(|_| Error::LockPoisoned("write buffer"))?
            .visible_range_changes(snapshot, start, start_exclusive, end)?;
        let base = match self.frozen_base()? {
            Some((base, head)) => base.layered_range_changes(head, start, start_exclusive, end)?,
            None => 0,
        };
        current.checked_add(base).ok_or(Error::FileTooLarge)
    }

    pub(in crate::infrastructure::file_store) fn merge_layered_range(
        &self,
        snapshot: Head,
        physical: Vec<(Vec<u8>, Vec<u8>)>,
        start: &[u8],
        start_exclusive: bool,
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let base = match self.frozen_base()? {
            Some((base, head)) => {
                let active_changes = self
                    .state
                    .read()
                    .map_err(|_| Error::LockPoisoned("write buffer"))?
                    .visible_range_changes(snapshot, start, start_exclusive, end)?;
                let base_limit = limit
                    .checked_add(active_changes)
                    .ok_or(Error::FileTooLarge)?;
                base.merge_layered_range(head, physical, start, start_exclusive, end, base_limit)?
            }
            None => physical,
        };
        self.state
            .read()
            .map_err(|_| Error::LockPoisoned("write buffer"))?
            .merge_range(snapshot, base, start, start_exclusive, end, limit)
    }

    pub(super) fn wal_generation_bytes(&self, wal_bytes: u64) -> u64 {
        wal_bytes.saturating_sub(self.wal_start)
    }
}
