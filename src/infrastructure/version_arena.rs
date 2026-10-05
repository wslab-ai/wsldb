use std::num::NonZeroUsize;
use std::ops::Deref;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::{Error, Result, TransactionId};

#[derive(Clone, Copy)]
pub(super) struct VersionId(NonZeroUsize);

impl VersionId {
    fn from_index(index: usize) -> Self {
        Self(NonZeroUsize::MIN.saturating_add(index))
    }

    fn index(self) -> usize {
        self.0.get() - 1
    }
}

#[derive(Clone, Copy)]
struct ValueRef {
    start: NonZeroUsize,
    len: usize,
}

pub(super) struct VersionedValue {
    pub(super) transaction_id: TransactionId,
    value: Option<ValueRef>,
    previous: Option<VersionId>,
}

impl VersionedValue {
    pub(super) const fn has_value(&self) -> bool {
        self.value.is_some()
    }
}

pub(super) struct VersionArenaData {
    versions: Vec<VersionedValue>,
    values: Vec<u8>,
}

impl VersionArenaData {
    pub(super) fn value<'a>(&'a self, version: &VersionedValue) -> Result<Option<&'a [u8]>> {
        let Some(value) = version.value else {
            return Ok(None);
        };
        let start = value.start.get() - 1;
        let end = start
            .checked_add(value.len)
            .ok_or(Error::InternalInvariant(
                "write-buffer value range overflow",
            ))?;
        self.values
            .get(start..end)
            .map(Some)
            .ok_or(Error::InternalInvariant(
                "write-buffer value range is invalid",
            ))
    }
}

impl Deref for VersionArenaData {
    type Target = [VersionedValue];

    fn deref(&self) -> &Self::Target {
        &self.versions
    }
}

pub(super) struct VersionArena {
    data: RwLock<VersionArenaData>,
}

impl VersionArena {
    pub(super) const fn new() -> Self {
        Self {
            data: RwLock::new(VersionArenaData {
                versions: Vec::new(),
                values: Vec::new(),
            }),
        }
    }

    pub(super) fn read(&self) -> Result<RwLockReadGuard<'_, VersionArenaData>> {
        self.data
            .read()
            .map_err(|_| Error::LockPoisoned("write-buffer version arena"))
    }

    /// Reserves transaction storage before the corresponding WAL is durable.
    pub(super) fn prepare_append(
        &self,
        additional_versions: usize,
        additional_value_bytes: usize,
    ) -> Result<()> {
        let mut data = self
            .data
            .write()
            .map_err(|_| Error::LockPoisoned("write-buffer version arena"))?;
        data.versions
            .try_reserve(additional_versions)
            .map_err(|_| Error::FileTooLarge)?;
        data.values
            .try_reserve(additional_value_bytes)
            .map_err(|_| Error::FileTooLarge)?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn capacities(&self) -> Result<(usize, usize)> {
        let data = self.read()?;
        Ok((data.versions.capacity(), data.values.capacity()))
    }

    /// Returns a writable arena after the WAL is durable.
    ///
    /// Publication after a successful WAL sync must not introduce an ordinary
    /// error path. As with the owning write-buffer lock, a poisoned guard is
    /// recovered so the durable record can deterministically finish applying.
    pub(super) fn write_durable(&self) -> RwLockWriteGuard<'_, VersionArenaData> {
        match self.data.write() {
            Ok(data) => data,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

pub(super) fn append_version(
    data: &mut VersionArenaData,
    transaction_id: TransactionId,
    value: Option<Vec<u8>>,
    previous: Option<VersionId>,
) -> VersionId {
    let id = VersionId::from_index(data.versions.len());
    let value = value.map(|value| {
        let reference = ValueRef {
            start: NonZeroUsize::MIN.saturating_add(data.values.len()),
            len: value.len(),
        };
        data.values.extend_from_slice(&value);
        reference
    });
    data.versions.push(VersionedValue {
        transaction_id,
        value,
        previous,
    });
    id
}

pub(super) fn latest_version(
    versions: &[VersionedValue],
    id: VersionId,
) -> Result<&VersionedValue> {
    versions
        .get(id.index())
        .ok_or(Error::InternalInvariant("write-buffer version is missing"))
}

pub(super) fn visible_version(
    versions: &[VersionedValue],
    mut id: VersionId,
    transaction_id: TransactionId,
) -> Result<Option<&VersionedValue>> {
    loop {
        let version = latest_version(versions, id)?;
        if version.transaction_id <= transaction_id {
            return Ok(Some(version));
        }
        let Some(previous) = version.previous else {
            return Ok(None);
        };
        if previous.index() >= id.index() {
            return Err(Error::InternalInvariant(
                "write-buffer version chain is not descending",
            ));
        }
        id = previous;
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::*;

    #[test]
    fn packed_ids_and_versions_keep_the_expected_layout() {
        assert_eq!(size_of::<Option<VersionId>>(), size_of::<usize>());
        assert_eq!(size_of::<VersionedValue>(), 4 * size_of::<usize>());
    }

    #[test]
    fn invalid_value_ranges_fail_closed() {
        let data = VersionArenaData {
            versions: Vec::new(),
            values: vec![1, 2, 3],
        };
        let version = VersionedValue {
            transaction_id: TransactionId::from_raw(1),
            value: Some(ValueRef {
                start: NonZeroUsize::MAX,
                len: 2,
            }),
            previous: None,
        };
        assert!(matches!(
            data.value(&version),
            Err(Error::InternalInvariant(_))
        ));
    }
}
