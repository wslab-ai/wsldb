use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::Arc;

use super::membership_filter::GrowingMembershipFilter;
use super::version_arena::{
    VersionArena, VersionArenaData, VersionId, VersionedValue, latest_version, visible_version,
};
#[cfg(test)]
use super::wal::WalLoad;
use crate::allocation::clone_bytes;
use crate::domain::{ChangeSet, Head};
use crate::{Error, Result, TransactionId, VerificationReport};

mod apply;
mod point_index;
mod verification;

type VersionMap = BTreeMap<Arc<[u8]>, Option<VersionId>>;
type VisibleEntry<'map, 'arena> = (&'map Arc<[u8]>, &'arena VersionedValue);

pub(crate) use point_index::{PointIndex, PointIndexSlot, PointUpdate};

pub(crate) struct WriteBuffer {
    checkpoint: Head,
    head: Head,
    versions: VersionMap,
    versioned_keys: usize,
    key_filter: Option<GrowingMembershipFilter>,
    arena: Arc<VersionArena>,
    wal_bytes: u64,
    consolidated: bool,
}

impl WriteBuffer {
    pub(crate) fn empty(checkpoint: Head) -> Self {
        Self::empty_with_filter(checkpoint, None)
    }

    pub(crate) fn empty_with_membership_filter(
        checkpoint: Head,
        key_filter: GrowingMembershipFilter,
    ) -> Self {
        Self::empty_with_filter(checkpoint, Some(key_filter))
    }

    fn empty_with_filter(checkpoint: Head, key_filter: Option<GrowingMembershipFilter>) -> Self {
        Self {
            checkpoint,
            head: checkpoint,
            versions: BTreeMap::new(),
            versioned_keys: 0,
            key_filter,
            arena: Arc::new(VersionArena::new()),
            wal_bytes: 0,
            consolidated: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_load(checkpoint: Head, load: WalLoad) -> Result<Self> {
        let mut buffer = Self::empty(checkpoint);
        buffer.wal_bytes = load.valid_bytes;
        for record in load.records {
            buffer.apply_record(record, false)?;
        }
        Ok(buffer)
    }

    pub(crate) const fn checkpoint(&self) -> Head {
        self.checkpoint
    }

    pub(crate) const fn head(&self) -> Head {
        self.head
    }

    pub(crate) const fn wal_bytes(&self) -> u64 {
        self.wal_bytes
    }

    pub(crate) fn set_wal_bytes(&mut self, bytes: u64) {
        self.wal_bytes = bytes;
    }

    pub(crate) fn key_filter_needs_growth(&self, additional_upper_bound: usize) -> Result<bool> {
        let Some(filter) = &self.key_filter else {
            return Ok(false);
        };
        let required = self
            .versioned_keys
            .checked_add(additional_upper_bound)
            .ok_or(Error::FileTooLarge)?;
        Ok(u64::try_from(required).map_err(|_| Error::FileTooLarge)? > filter.capacity())
    }

    pub(crate) fn prepare_key_filter(&mut self, additional_keys: u64) -> Result<()> {
        let existing = u64::try_from(self.versioned_keys).map_err(|_| Error::FileTooLarge)?;
        let required = existing
            .checked_add(additional_keys)
            .ok_or(Error::FileTooLarge)?;
        if let Some(filter) = &mut self.key_filter {
            filter.ensure_capacity(
                required,
                self.versions
                    .iter()
                    .filter_map(|(key, version)| version.map(|_| key.as_ref())),
            )?;
        }
        Ok(())
    }

    pub(crate) fn prepare_apply<'a, I>(&mut self, changes: I) -> Result<usize>
    where
        I: Clone + IntoIterator<Item = &'a ChangeSet>,
    {
        let mut mutations = 0_usize;
        let mut value_bytes = 0_usize;
        for changes in changes.clone() {
            mutations = mutations
                .checked_add(changes.len())
                .ok_or(Error::FileTooLarge)?;
            for (_, value) in changes.sorted_iter() {
                value_bytes = value_bytes
                    .checked_add(value.map_or(0, <[u8]>::len))
                    .ok_or(Error::FileTooLarge)?;
            }
        }
        self.arena.prepare_append(mutations, value_bytes)?;
        for changes in changes {
            for (key, _) in changes.shared_sorted_iter() {
                self.versions.entry(Arc::clone(key)).or_insert(None);
            }
        }
        Ok(mutations)
    }

    /// Removes keys whose ordered-map nodes were prepared for a WAL append
    /// that did not complete. Such keys are never visible to readers.
    pub(crate) fn discard_prepared(&mut self) {
        self.versions.retain(|_, version| version.is_some());
    }

    pub(crate) const fn is_consolidated(&self) -> bool {
        self.consolidated
    }

    fn contains_key(&self, key: &[u8]) -> bool {
        self.versions.get(key).is_some_and(Option::is_some)
    }

    #[cfg(test)]
    pub(crate) fn visible_value(
        &self,
        snapshot: Head,
        key: &[u8],
    ) -> Result<Option<Option<Vec<u8>>>> {
        self.validate_snapshot(snapshot)?;
        let Some(version) = self.versions.get(key).copied().flatten() else {
            return Ok(None);
        };
        let arena = self.arena.read()?;
        let Some(version) = visible_version(&arena, version, snapshot.transaction_id())? else {
            return Ok(None);
        };
        Ok(Some(arena.value(version)?.map(clone_bytes).transpose()?))
    }

    pub(crate) fn visible_presence(&self, snapshot: Head, key: &[u8]) -> Result<Option<bool>> {
        self.validate_snapshot(snapshot)?;
        if self
            .key_filter
            .as_ref()
            .is_some_and(|filter| !filter.may_contain(key))
        {
            return Ok(None);
        }
        let Some(version) = self.versions.get(key).copied().flatten() else {
            return Ok(None);
        };
        let arena = self.arena.read()?;
        Ok(visible_version(&arena, version, snapshot.transaction_id())?
            .map(VersionedValue::has_value))
    }

    #[cfg(test)]
    fn key_filter_may_contain(&self, key: &[u8]) -> Option<bool> {
        self.key_filter
            .as_ref()
            .map(|filter| filter.may_contain(key))
    }

    #[cfg(test)]
    fn key_filter_capacity(&self) -> Option<u64> {
        self.key_filter
            .as_ref()
            .map(GrowingMembershipFilter::capacity)
    }

    pub(crate) fn merge_scan(
        &self,
        snapshot: Head,
        base: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.validate_snapshot(snapshot)?;
        let arena = self.arena.read()?;
        let mut base = base.into_iter().peekable();
        let mut buffered = self.versions.iter();
        let mut buffered_entry = next_visible(&mut buffered, &arena, snapshot.transaction_id())?;
        let mut entries = Vec::new();
        entries
            .try_reserve(base.len().saturating_add(self.versioned_keys))
            .map_err(|_| Error::FileTooLarge)?;
        while let (Some((base_key, _)), Some((buffered_key, _))) = (base.peek(), buffered_entry) {
            match base_key.as_slice().cmp(buffered_key.as_ref()) {
                Ordering::Less => entries.push(
                    base.next()
                        .ok_or(Error::InternalInvariant("base scan entry disappeared"))?,
                ),
                Ordering::Equal => {
                    let _ = base
                        .next()
                        .ok_or(Error::InternalInvariant("base scan entry disappeared"))?;
                    let (key, version) = buffered_entry
                        .ok_or(Error::InternalInvariant("buffered scan entry disappeared"))?;
                    push_visible_entry(&mut entries, key, version, &arena)?;
                    buffered_entry =
                        next_visible(&mut buffered, &arena, snapshot.transaction_id())?;
                }
                Ordering::Greater => {
                    let (key, version) = buffered_entry
                        .ok_or(Error::InternalInvariant("buffered scan entry disappeared"))?;
                    push_visible_entry(&mut entries, key, version, &arena)?;
                    buffered_entry =
                        next_visible(&mut buffered, &arena, snapshot.transaction_id())?;
                }
            }
        }
        entries.extend(base);
        if let Some((key, version)) = buffered_entry {
            push_visible_entry(&mut entries, key, version, &arena)?;
        }
        while let Some((key, version)) =
            next_visible(&mut buffered, &arena, snapshot.transaction_id())?
        {
            push_visible_entry(&mut entries, key, version, &arena)?;
        }
        if entries.len() as u64 != snapshot.entry_count() {
            return Err(Error::Corrupt(
                "buffered entry count does not match its snapshot".into(),
            ));
        }
        Ok(entries)
    }

    pub(crate) fn merge_range(
        &self,
        snapshot: Head,
        base: Vec<(Vec<u8>, Vec<u8>)>,
        start: &[u8],
        start_exclusive: bool,
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.validate_snapshot(snapshot)?;
        let arena = self.arena.read()?;
        let mut base = base.into_iter().peekable();
        let mut buffered = version_range(&self.versions, start, start_exclusive, end);
        let mut buffered_entry = next_visible(&mut buffered, &arena, snapshot.transaction_id())?;
        let mut entries = Vec::new();
        entries
            .try_reserve(limit.min(1024))
            .map_err(|_| Error::FileTooLarge)?;
        while entries.len() < limit {
            match (base.peek(), buffered_entry) {
                (Some((base_key, _)), Some((buffered_key, _))) => {
                    match base_key.as_slice().cmp(buffered_key.as_ref()) {
                        Ordering::Less => push_owned_entry(&mut entries, &mut base)?,
                        Ordering::Equal => {
                            let _ = base
                                .next()
                                .ok_or(Error::InternalInvariant("base range entry disappeared"))?;
                            let (key, version) = buffered_entry.ok_or(Error::InternalInvariant(
                                "buffered range entry disappeared",
                            ))?;
                            push_visible_entry(&mut entries, key, version, &arena)?;
                            buffered_entry =
                                next_visible(&mut buffered, &arena, snapshot.transaction_id())?;
                        }
                        Ordering::Greater => {
                            let (key, version) = buffered_entry.ok_or(Error::InternalInvariant(
                                "buffered range entry disappeared",
                            ))?;
                            push_visible_entry(&mut entries, key, version, &arena)?;
                            buffered_entry =
                                next_visible(&mut buffered, &arena, snapshot.transaction_id())?;
                        }
                    }
                }
                (Some(_), None) => push_owned_entry(&mut entries, &mut base)?,
                (None, Some((key, version))) => {
                    push_visible_entry(&mut entries, key, version, &arena)?;
                    buffered_entry =
                        next_visible(&mut buffered, &arena, snapshot.transaction_id())?;
                }
                (None, None) => break,
            }
        }
        Ok(entries)
    }

    pub(crate) fn visible_range_changes(
        &self,
        snapshot: Head,
        start: &[u8],
        start_exclusive: bool,
        end: Option<&[u8]>,
    ) -> Result<usize> {
        self.validate_snapshot(snapshot)?;
        let arena = self.arena.read()?;
        let mut changes = 0_usize;
        for (_, version) in version_range(&self.versions, start, start_exclusive, end) {
            if let Some(version) = *version
                && visible_version(&arena, version, snapshot.transaction_id())?.is_some()
            {
                changes = changes.checked_add(1).ok_or(Error::FileTooLarge)?;
            }
        }
        Ok(changes)
    }

    pub(crate) fn latest_changes(&self) -> Result<ChangeSet> {
        let mut changes = ChangeSet::new();
        let arena = self.arena.read()?;
        for (key, version) in &self.versions {
            let Some(version) = *version else {
                continue;
            };
            let version = latest_version(&arena, version)?;
            let value = arena.value(version)?.map(clone_bytes).transpose()?;
            changes.insert_shared(Arc::clone(key), value);
        }
        Ok(changes)
    }

    pub(crate) fn latest_are_puts(&self) -> Result<bool> {
        let arena = self.arena.read()?;
        for version in self.versions.values() {
            let Some(version) = *version else {
                continue;
            };
            if !latest_version(&arena, version)?.has_value() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) fn visit_latest(
        &self,
        mut visitor: impl FnMut(&[u8], Option<&[u8]>) -> Result<()>,
    ) -> Result<()> {
        let arena = self.arena.read()?;
        for (key, version) in &self.versions {
            let Some(version) = *version else {
                continue;
            };
            let version = latest_version(&arena, version)?;
            visitor(key, arena.value(version)?)?;
        }
        Ok(())
    }

    pub(crate) fn snapshot_rebased(
        &self,
        checkpoint: Head,
        mut key_filter: GrowingMembershipFilter,
        wal_bytes: u64,
    ) -> Result<Self> {
        if checkpoint.transaction_id() != self.checkpoint.transaction_id()
            || checkpoint.entry_count() != self.checkpoint.entry_count()
            || self.head.transaction_id() <= checkpoint.transaction_id()
        {
            return Err(Error::InternalInvariant(
                "write-buffer snapshot checkpoint is inconsistent",
            ));
        }
        key_filter.ensure_capacity(
            u64::try_from(self.versioned_keys).map_err(|_| Error::FileTooLarge)?,
            self.versions
                .iter()
                .filter_map(|(key, version)| version.map(|_| key.as_ref())),
        )?;
        for (key, version) in &self.versions {
            if version.is_some() {
                key_filter.insert(key);
            }
        }
        Ok(Self {
            checkpoint,
            head: Head::new(
                self.head.transaction_id(),
                checkpoint.root(),
                checkpoint.last_page(),
                self.head.entry_count(),
            ),
            versions: self.versions.clone(),
            versioned_keys: self.versioned_keys,
            key_filter: Some(key_filter),
            arena: Arc::clone(&self.arena),
            wal_bytes,
            consolidated: true,
        })
    }

    pub(crate) fn verification(
        &self,
        reader: &impl super::tree::PageReader,
        snapshot: Head,
    ) -> Result<VerificationReport> {
        verification::verify(self, reader, snapshot)
    }

    fn validate_snapshot(&self, snapshot: Head) -> Result<()> {
        if snapshot.transaction_id() < self.checkpoint.transaction_id()
            || snapshot.transaction_id() > self.head.transaction_id()
            || snapshot.root() != self.checkpoint.root()
            || snapshot.last_page() != self.checkpoint.last_page()
        {
            return Err(Error::InternalInvariant(
                "snapshot does not belong to this write buffer",
            ));
        }
        Ok(())
    }
}

fn push_visible_entry(
    entries: &mut Vec<(Vec<u8>, Vec<u8>)>,
    key: &Arc<[u8]>,
    version: &VersionedValue,
    arena: &VersionArenaData,
) -> Result<()> {
    if let Some(value) = arena.value(version)? {
        entries.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
        entries.push((clone_bytes(key)?, clone_bytes(value)?));
    }
    Ok(())
}

fn push_owned_entry(
    entries: &mut Vec<(Vec<u8>, Vec<u8>)>,
    source: &mut impl Iterator<Item = (Vec<u8>, Vec<u8>)>,
) -> Result<()> {
    entries.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
    entries.push(
        source
            .next()
            .ok_or(Error::InternalInvariant("base range entry disappeared"))?,
    );
    Ok(())
}

fn next_visible<'map, 'arena>(
    entries: &mut impl Iterator<Item = (&'map Arc<[u8]>, &'map Option<VersionId>)>,
    arena: &'arena VersionArenaData,
    transaction_id: TransactionId,
) -> Result<Option<VisibleEntry<'map, 'arena>>> {
    for (key, version) in entries {
        let Some(version) = *version else {
            continue;
        };
        if let Some(version) = visible_version(arena, version, transaction_id)? {
            return Ok(Some((key, version)));
        }
    }
    Ok(None)
}

fn version_range<'a>(
    entries: &'a VersionMap,
    start: &'a [u8],
    start_exclusive: bool,
    end: Option<&'a [u8]>,
) -> impl Iterator<Item = (&'a Arc<[u8]>, &'a Option<VersionId>)> {
    let start = if start_exclusive {
        Bound::Excluded(start)
    } else {
        Bound::Included(start)
    };
    let end = end.map_or(Bound::Unbounded, Bound::Excluded);
    entries.range::<[u8], _>((start, end))
}

#[cfg(test)]
mod tests;
