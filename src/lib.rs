#![doc = include_str!("../README.md")]

mod allocation;
mod application;
mod domain;
mod error;
mod infrastructure;

use std::path::Path;
use std::sync::Arc;

pub use application::{
    CheckpointStats, Database, DatabaseOptions, DatabaseStats, EntryIter, LatencyStats,
    MAX_GROUP_TRANSACTIONS, OperationalStats, PageCacheStats, ReadTransaction, WriteBatch,
    WriteStats, WriteTransaction,
};
pub use domain::{
    BackupReport, CompactionReport, MAX_KEY_BYTES, MAX_VALUE_BYTES, TransactionId,
    VerificationReport,
};
pub use error::{Error, Result};

use infrastructure::FileStorageEngine;

impl Database {
    /// Opens an existing database or creates a new one at `path`.
    ///
    /// Parent directories are not created automatically. Commits through
    /// separately opened handles and processes are serialized by a file lock.
    /// Existing symbolic links are resolved to one canonical path. On Unix,
    /// database files with multiple hard links are rejected because distinct
    /// names cannot safely share the stable pathname lock.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(path, DatabaseOptions::default())
    }

    /// Opens a database with explicit runtime options.
    ///
    /// Path canonicalization and hard-link restrictions are identical to
    /// [`Self::open`].
    pub fn open_with_options(path: impl AsRef<Path>, options: DatabaseOptions) -> Result<Self> {
        let (store, head) = FileStorageEngine::open(
            path.as_ref(),
            options.page_cache_capacity,
            options.write_buffer_bytes,
            options.verify_on_open,
        )?;
        Ok(Self::from_store(Arc::new(store), head))
    }

    /// Atomically replaces `path` with a compacted snapshot.
    ///
    /// Readers that already hold the old file remain valid. Existing database
    /// handles become stale and their writers return [`Error::WriteConflict`];
    /// reopen the path after this operation before issuing more work.
    /// [`Error::PublicationOutcomeUnknown`] means the replacement may already
    /// be active and the path must be inspected before retrying.
    pub fn compact_file(path: impl AsRef<Path>) -> Result<CompactionReport> {
        FileStorageEngine::compact_file(path.as_ref())
    }
}
