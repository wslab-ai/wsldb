use super::mutation::{validate_key, validate_value};
use crate::Result;
use crate::allocation::clone_bytes;
use crate::domain::ChangeSet;

/// Maximum number of logical transactions in one physical group-commit frame.
///
/// Automatic group commit splits a larger pending queue into ordered groups of
/// this size. [`crate::Database::commit_group`] rejects a larger explicit
/// group before acquiring the writer lock.
pub const MAX_GROUP_TRANSACTIONS: usize = 65_536;

/// An ordered set of mutations prepared for an explicit group commit.
///
/// Repeated mutations of one key retain only the final value within this
/// batch. Batches do not read database state; use [`crate::WriteTransaction`]
/// when a mutation depends on a read.
#[derive(Clone)]
pub struct WriteBatch {
    changes: ChangeSet,
}

impl WriteBatch {
    /// Creates an empty batch.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            changes: ChangeSet::new(),
        }
    }

    /// Returns the number of distinct keys mutated by this batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// Returns whether this batch has no mutations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Adds or replaces a key/value pair.
    ///
    /// Keys must be non-empty and at most [`crate::MAX_KEY_BYTES`] bytes.
    /// Values may be empty and are limited to [`crate::MAX_VALUE_BYTES`] bytes.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        validate_key(key)?;
        validate_value(value)?;
        self.changes.insert(key, Some(clone_bytes(value)?));
        Ok(())
    }

    /// Marks a key for deletion.
    ///
    /// Keys must be non-empty and at most [`crate::MAX_KEY_BYTES`] bytes.
    pub fn delete(&mut self, key: &[u8]) -> Result<()> {
        validate_key(key)?;
        self.changes.insert(key, None);
        Ok(())
    }

    pub(crate) fn into_changes(self) -> ChangeSet {
        self.changes
    }
}

impl Default for WriteBatch {
    fn default() -> Self {
        Self::new()
    }
}
