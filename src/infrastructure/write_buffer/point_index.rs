use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};

use super::WriteBuffer;
use crate::allocation::clone_bytes;
use crate::domain::ChangeSet;
use crate::infrastructure::version_arena::{VersionArena, VersionId, visible_version};
use crate::{Error, Result, TransactionId};

const POINT_SHARDS: usize = 64;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
type PointMap = HashMap<Arc<[u8]>, VersionId>;

pub(crate) struct PointUpdate {
    pub(super) key: Arc<[u8]>,
    pub(super) version: VersionId,
}

pub(crate) struct PointIndex {
    shards: [RwLock<PointMap>; POINT_SHARDS],
    arena: Arc<VersionArena>,
}

pub(crate) struct PointIndexSlot {
    index: OnceLock<PointIndex>,
    initialization: Mutex<()>,
}

impl PointIndexSlot {
    pub(crate) const fn new() -> Self {
        Self {
            index: OnceLock::new(),
            initialization: Mutex::new(()),
        }
    }

    pub(crate) fn get(&self) -> Option<&PointIndex> {
        self.index.get()
    }

    pub(crate) fn get_or_initialize(
        &self,
        build: impl FnOnce() -> Result<PointIndex>,
    ) -> Result<&PointIndex> {
        if let Some(index) = self.index.get() {
            return Ok(index);
        }
        let _initialization = self
            .initialization
            .lock()
            .map_err(|_| Error::LockPoisoned("point-index initialization"))?;
        if self.index.get().is_none() && self.index.set(build()?).is_err() {
            return Err(Error::InternalInvariant(
                "point index was initialized without its guard",
            ));
        }
        self.index
            .get()
            .ok_or(Error::InternalInvariant("point index is missing"))
    }

    /// Prevents a first reader from building the index from a pre-commit
    /// buffer while its WAL synchronization is in flight.
    pub(crate) fn lock_if_empty(&self) -> Result<Option<MutexGuard<'_, ()>>> {
        if self.index.get().is_some() {
            return Ok(None);
        }
        let initialization = self
            .initialization
            .lock()
            .map_err(|_| Error::LockPoisoned("point-index initialization"))?;
        if self.index.get().is_some() {
            Ok(None)
        } else {
            Ok(Some(initialization))
        }
    }
}

impl PointIndex {
    fn empty(arena: Arc<VersionArena>) -> Self {
        Self {
            shards: std::array::from_fn(|_| RwLock::new(HashMap::new())),
            arena,
        }
    }

    pub(crate) fn empty_for(buffer: &WriteBuffer) -> Self {
        Self::empty(Arc::clone(&buffer.arena))
    }

    pub(crate) fn from_buffer(buffer: &WriteBuffer) -> Result<Self> {
        let mut index = Self::empty_for(buffer);
        let mut shard_entries = [0_usize; POINT_SHARDS];
        for (key, version) in &buffer.versions {
            if version.is_none() {
                continue;
            }
            let shard = shard_index(key);
            shard_entries[shard] = shard_entries[shard]
                .checked_add(1)
                .ok_or(Error::FileTooLarge)?;
        }
        for (shard, entries) in shard_entries.into_iter().enumerate() {
            index
                .shard_mut_durable(shard)
                .try_reserve(entries)
                .map_err(|_| Error::FileTooLarge)?;
        }
        for (key, version) in &buffer.versions {
            let Some(version) = *version else {
                continue;
            };
            index
                .shard_mut_durable(shard_index(key))
                .insert(Arc::clone(key), version);
        }
        Ok(index)
    }

    pub(crate) fn preflight(&self, changes: &ChangeSet, ordered: &WriteBuffer) -> Result<()> {
        let mut ordered_presence = Vec::new();
        ordered_presence
            .try_reserve_exact(changes.len())
            .map_err(|_| Error::FileTooLarge)?;
        ordered_presence.extend(
            changes
                .sorted_iter()
                .map(|(key, _)| ordered.contains_key(key)),
        );
        self.preflight_known(changes, &ordered_presence)
    }

    pub(crate) fn preflight_known(
        &self,
        changes: &ChangeSet,
        ordered_presence: &[bool],
    ) -> Result<()> {
        if changes.len() != ordered_presence.len() {
            return Err(Error::InternalInvariant(
                "point-index preflight length is invalid",
            ));
        }
        let mut affected: [Vec<(&[u8], bool)>; POINT_SHARDS] = std::array::from_fn(|_| Vec::new());
        for ((key, _), present) in changes.sorted_iter().zip(ordered_presence.iter().copied()) {
            let shard = &mut affected[shard_index(key)];
            shard.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
            shard.push((key, present));
        }
        for (shard, keys) in affected.into_iter().enumerate() {
            if keys.is_empty() {
                continue;
            }
            let mut entries = self.shards[shard]
                .write()
                .map_err(|_| Error::LockPoisoned("write-buffer point index"))?;
            for (key, present) in &keys {
                if entries.contains_key(*key) != *present {
                    return Err(Error::InternalInvariant(
                        "write-buffer point index disagrees with ordered index",
                    ));
                }
            }
            let new_keys = keys.iter().filter(|(_, present)| !present).count();
            entries
                .try_reserve(new_keys)
                .map_err(|_| Error::FileTooLarge)?;
        }
        Ok(())
    }

    pub(crate) fn preflight_one(&self, key: &[u8], present: bool) -> Result<()> {
        let entries = self.shards[shard_index(key)]
            .write()
            .map_err(|_| Error::LockPoisoned("write-buffer point index"))?;
        if entries.contains_key(key) != present {
            return Err(Error::InternalInvariant(
                "write-buffer point index disagrees with ordered index",
            ));
        }
        if !present {
            let mut entries = entries;
            entries.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn capacity_for(&self, key: &[u8]) -> Result<usize> {
        self.shards[shard_index(key)]
            .read()
            .map(|entries| entries.capacity())
            .map_err(|_| Error::LockPoisoned("write-buffer point index"))
    }

    pub(crate) fn apply_durable(&self, mut updates: Vec<PointUpdate>) {
        updates.sort_unstable_by_key(|update| shard_index(&update.key));
        let mut updates = updates.into_iter().peekable();
        while let Some(first) = updates.next() {
            let shard = shard_index(&first.key);
            let mut entries = match self.shards[shard].write() {
                Ok(entries) => entries,
                Err(poisoned) => poisoned.into_inner(),
            };
            entries.insert(first.key, first.version);
            while let Some(update) = updates.next_if(|update| shard_index(&update.key) == shard) {
                entries.insert(update.key, update.version);
            }
        }
    }

    pub(crate) fn visible_value(
        &self,
        transaction_id: TransactionId,
        key: &[u8],
    ) -> Result<Option<Option<Vec<u8>>>> {
        let entries = self
            .shard(key)
            .read()
            .map_err(|_| Error::LockPoisoned("write-buffer point index"))?;
        let version = entries.get(key).copied();
        drop(entries);
        let Some(version) = version else {
            return Ok(None);
        };
        let arena = self.arena.read()?;
        let Some(version) = visible_version(&arena, version, transaction_id)? else {
            return Ok(None);
        };
        Ok(Some(arena.value(version)?.map(clone_bytes).transpose()?))
    }

    fn shard(&self, key: &[u8]) -> &RwLock<PointMap> {
        &self.shards[shard_index(key)]
    }

    fn shard_mut_durable(&mut self, shard: usize) -> &mut PointMap {
        match self.shards[shard].get_mut() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn shard_index(key: &[u8]) -> usize {
    let hash = key.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    });
    hash as usize % POINT_SHARDS
}
