use std::sync::{Arc, MutexGuard, RwLock};

use super::mutation::{validate_key, validate_value};
use super::{DatabaseState, StorageEngine};
use crate::allocation::clone_bytes;
use crate::domain::{ChangeSet, Head};
use crate::{Error, Result, TransactionId};

/// Iterator over owned entries in deterministic bytewise key order.
pub struct EntryIter {
    inner: std::vec::IntoIter<(Vec<u8>, Vec<u8>)>,
}

impl EntryIter {
    fn new(entries: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        Self {
            inner: entries.into_iter(),
        }
    }
}

impl Iterator for EntryIter {
    type Item = (Vec<u8>, Vec<u8>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for EntryIter {}
impl std::iter::FusedIterator for EntryIter {}

/// An immutable view of one committed tree root.
pub struct ReadTransaction {
    store: Arc<dyn StorageEngine>,
    head: Head,
}

impl ReadTransaction {
    pub(crate) fn new(store: Arc<dyn StorageEngine>, head: Head) -> Self {
        Self { store, head }
    }

    /// Returns the transaction identifier of this snapshot.
    #[must_use]
    pub fn transaction_id(&self) -> TransactionId {
        self.head.transaction_id()
    }

    /// Returns an owned value for `key` in this snapshot.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.store.get(self.head, key)
    }

    /// Returns the number of entries in this snapshot.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.head.entry_count()
    }

    /// Returns whether this snapshot contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.head.entry_count() == 0
    }

    /// Reads entries in bytewise key order into an owned iterator.
    pub fn iter(&self) -> Result<EntryIter> {
        self.store.scan(self.head).map(EntryIter::new)
    }

    /// Reads at most `limit` entries from `start` (inclusive) to `end`
    /// (exclusive) in bytewise key order.
    ///
    /// Passing `None` for `end` scans toward the end of the database. A zero
    /// limit or an end key not greater than `start` returns an empty iterator.
    pub fn range(&self, start: &[u8], end: Option<&[u8]>, limit: usize) -> Result<EntryIter> {
        self.store
            .scan_range(self.head, start, false, end, limit)
            .map(EntryIter::new)
    }

    /// Reads the next page of entries strictly after `after` in bytewise key order.
    ///
    /// Passing `None` starts at the first key. `end` remains exclusive. This
    /// method is intended for bounded pagination of a stable read transaction:
    /// pass the last key returned by one page as `after` for the next page.
    /// A zero limit or an end key not greater than `after` returns an empty
    /// iterator.
    pub fn range_after(
        &self,
        after: Option<&[u8]>,
        end: Option<&[u8]>,
        limit: usize,
    ) -> Result<EntryIter> {
        let (start, start_exclusive) = after.map_or((&[][..], false), |key| (key, true));
        self.store
            .scan_range(self.head, start, start_exclusive, end, limit)
            .map(EntryIter::new)
    }
}

/// A serialized write transaction with read-your-writes behavior.
///
/// Dropping this value without calling [`Self::commit`] aborts all mutations.
pub struct WriteTransaction<'database> {
    state: &'database RwLock<DatabaseState>,
    store: Arc<dyn StorageEngine>,
    _writer_guard: MutexGuard<'database, ()>,
    base: Head,
    changes: ChangeSet,
}

impl<'database> WriteTransaction<'database> {
    pub(crate) fn new(
        state: &'database RwLock<DatabaseState>,
        store: Arc<dyn StorageEngine>,
        writer_guard: MutexGuard<'database, ()>,
        base: Head,
    ) -> Self {
        Self {
            state,
            store,
            _writer_guard: writer_guard,
            base,
            changes: ChangeSet::new(),
        }
    }

    /// Returns the value visible after applying this transaction's mutations.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        match self.changes.get(key) {
            Some(Some(value)) => clone_bytes(value).map(Some),
            Some(None) => Ok(None),
            None => self.store.get(self.base, key),
        }
    }

    /// Adds or replaces a key/value pair in this transaction.
    ///
    /// Keys must be non-empty and at most [`crate::MAX_KEY_BYTES`] bytes.
    /// Values may be empty and are limited to [`crate::MAX_VALUE_BYTES`] bytes.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        validate_key(key)?;
        validate_value(value)?;
        self.changes.insert(key, Some(clone_bytes(value)?));
        Ok(())
    }

    /// Marks a key for deletion and returns whether it currently exists.
    ///
    /// Keys must be non-empty and at most [`crate::MAX_KEY_BYTES`] bytes.
    pub fn delete(&mut self, key: &[u8]) -> Result<bool> {
        validate_key(key)?;
        let existed = self.get(key)?.is_some();
        self.changes.insert(key, None);
        Ok(existed)
    }

    /// Publishes the transaction durably and returns its new identifier.
    ///
    /// [`Error::CommitOutcomeUnknown`] means publication began but an I/O
    /// failure prevented confirmation. Refresh or reopen the database and
    /// inspect its transaction ID before deciding whether to retry.
    pub fn commit(self) -> Result<TransactionId> {
        let expected = self.base.transaction_id().next()?;
        let committed = self
            .store
            .commit(self.base, self.changes)?
            .require_transaction_id(expected)?;
        let mut state = self
            .state
            .write()
            .map_err(|_| Error::LockPoisoned("database state"))?;
        if let Some(store) = committed.store {
            state.store = store;
        }
        state.head = committed.head;
        Ok(committed.head.transaction_id())
    }

    /// Explicitly aborts the transaction. Dropping it has the same effect.
    pub fn abort(self) {}
}
