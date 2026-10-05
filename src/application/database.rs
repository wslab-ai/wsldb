use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use super::group_commit::{BatchCommitCoordinator, WaitAction, complete_group};
use super::{MAX_GROUP_TRANSACTIONS, ReadTransaction, StorageEngine, WriteBatch, WriteTransaction};
use crate::domain::Head;
use crate::{BackupReport, CompactionReport, Error, Result, TransactionId, VerificationReport};

struct DatabaseInner {
    state: RwLock<DatabaseState>,
    writer: Mutex<()>,
    batch_commits: BatchCommitCoordinator,
}

pub(crate) struct DatabaseState {
    pub(crate) head: Head,
    pub(crate) store: Arc<dyn StorageEngine>,
}

/// A cloneable handle to an open WSLDB database.
#[derive(Clone)]
pub struct Database {
    inner: Arc<DatabaseInner>,
}

/// Summary of the currently committed tree version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DatabaseStats {
    /// Identifier of the current committed transaction.
    pub transaction_id: TransactionId,
    /// Number of key/value entries in the current tree.
    pub entries: u64,
    /// Highest page ID published by the current commit.
    pub last_page: u64,
}

/// Configuration used when opening a database.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseOptions {
    pub(crate) page_cache_capacity: usize,
    pub(crate) write_buffer_bytes: u64,
    pub(crate) verify_on_open: bool,
}

impl DatabaseOptions {
    /// Creates options with production-oriented defaults.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            page_cache_capacity: 4096,
            write_buffer_bytes: 0,
            verify_on_open: false,
        }
    }

    /// Sets the maximum number of decoded pages retained in the shared cache.
    ///
    /// Reader threads may additionally retain up to 64 decoded pages in a
    /// local cache. Newly bulk-created leaf and overflow pages are admitted on
    /// their first read; hot replacement and branch pages can be admitted at
    /// commit. A capacity of zero disables the shared cache.
    #[must_use]
    pub const fn page_cache_capacity(mut self, pages: usize) -> Self {
        self.page_cache_capacity = pages;
        self
    }

    /// Enables durable write buffering and sets its checkpoint threshold.
    ///
    /// A zero threshold keeps the direct copy-on-write commit path. Buffered
    /// commits remain fully durable and are recovered automatically on reopen.
    /// If zero is selected while an existing WAL still needs recovery, WSLDB
    /// reads it through a temporary buffer; group commit stays disabled, and a
    /// flush or completed checkpoint returns the handle to the direct path.
    /// Dropping the last owner of a buffered storage generation waits for any
    /// checkpoint worker for that generation, so checkpoint I/O cannot remain
    /// detached after the owner is destroyed.
    #[must_use]
    pub const fn write_buffer_bytes(mut self, bytes: u64) -> Self {
        self.write_buffer_bytes = bytes;
        self
    }

    /// Enables full logical verification before an open returns.
    ///
    /// The default fast-open path validates metadata, the selected root, and
    /// every WAL record, while deeper page validation remains lazy. Enabling
    /// this option additionally scans the complete reachable tree and all
    /// overflow values. It can make opening proportional to database size but
    /// ensures deep corruption is handled before a handle is returned. If the
    /// WAL was empty before recovery, WSLDB may return the newest older
    /// metadata snapshot that passes full verification. A nonempty WAL forbids
    /// this fallback so newer durable transactions are never discarded.
    /// Fallback does not rewrite the damaged generation; treat it as a recovery
    /// view. Writes and active maintenance fail with [`Error::WriteConflict`],
    /// while [`Database::backup`] and [`Database::compact_to`] can export that
    /// verified view into a new database if the source generation is unchanged.
    #[must_use]
    pub const fn verify_on_open(mut self, enabled: bool) -> Self {
        self.verify_on_open = enabled;
        self
    }
}

impl Default for DatabaseOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime counters for the decoded page cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PageCacheStats {
    /// Configured maximum number of pages in the shared cache.
    pub capacity: usize,
    /// Current number of pages in the shared cache.
    pub resident_pages: usize,
    /// Number of page reads served from the cache.
    pub hits: u64,
    /// Number of page reads that required storage access.
    pub misses: u64,
}

/// Aggregate timing counters for a durable operation.
///
/// Percentiles are conservative upper bounds from a fixed logarithmic
/// histogram with 16 buckets per power of two. A non-zero estimate is no more
/// than 6.25% above the recorded value at the requested rank. Empty
/// histograms report zero for every percentile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LatencyStats {
    /// Number of completed operations.
    pub operations: u64,
    /// Sum of recorded operation durations.
    pub total_nanos: u64,
    /// Duration of the most recently completed operation.
    pub last_nanos: u64,
    /// Largest recorded operation duration.
    pub max_nanos: u64,
    /// Approximate median operation duration.
    pub p50_nanos: u64,
    /// Approximate 95th-percentile operation duration.
    pub p95_nanos: u64,
    /// Approximate 99th-percentile operation duration.
    pub p99_nanos: u64,
    /// Approximate 99.9th-percentile operation duration.
    pub p999_nanos: u64,
}

/// Process-local outcome counters for primary-tree checkpoints.
///
/// Reading these counters does not verify or scan the database and is suitable
/// for lightweight monitoring. WAL snapshot consolidation is not a primary-
/// tree checkpoint and is therefore excluded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CheckpointStats {
    /// Primary-tree checkpoints whose preparation started in this process.
    pub started: u64,
    /// Primary-tree checkpoints currently preparing or awaiting publication.
    pub in_progress: u64,
    /// End-to-end lifetime of successfully published primary-tree checkpoints.
    pub successful: LatencyStats,
    /// Automatic primary-tree checkpoints that could not be completed.
    ///
    /// The triggering transaction remains durable in the write buffer.
    pub failures: u64,
    /// Data-page and metadata bytes written by successful checkpoints.
    pub database_bytes: u64,
}

/// Bytes accepted and emitted by successful commits in this process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct WriteStats {
    /// Final key and value bytes requested by committed mutations.
    ///
    /// Deletes contribute their key bytes. Multiple mutations of one key in a
    /// transaction contribute only the final mutation.
    pub logical_mutation_bytes: u64,
    /// Complete immutable data pages written by successful commits.
    pub data_page_bytes: u64,
    /// Leaf-page portion of [`Self::data_page_bytes`].
    pub leaf_page_bytes: u64,
    /// Branch-page portion of [`Self::data_page_bytes`].
    pub branch_page_bytes: u64,
    /// Overflow-page portion of [`Self::data_page_bytes`].
    pub overflow_page_bytes: u64,
    /// Bytes appended and synced to the durable write buffer.
    pub write_buffer_bytes: u64,
    /// Metadata pages written to publish successful commits.
    pub metadata_bytes: u64,
}

impl WriteStats {
    /// Returns data-page and metadata bytes written by the database engine.
    #[must_use]
    pub const fn database_bytes(self) -> u64 {
        self.data_page_bytes
            .saturating_add(self.write_buffer_bytes)
            .saturating_add(self.metadata_bytes)
    }
}

fn consecutive_transaction_ids(base: TransactionId, count: usize) -> Result<Vec<TransactionId>> {
    let count_u64 = u64::try_from(count).map_err(|_| Error::TransactionIdExhausted)?;
    base.value()
        .checked_add(count_u64)
        .ok_or(Error::TransactionIdExhausted)?;
    let mut transaction_ids = Vec::new();
    transaction_ids
        .try_reserve_exact(count)
        .map_err(|_| Error::FileTooLarge)?;
    let mut next = base;
    for _ in 0..count {
        next = next.next()?;
        transaction_ids.push(next);
    }
    Ok(transaction_ids)
}

fn validate_group_limit(count: usize) -> Result<()> {
    if count > MAX_GROUP_TRANSACTIONS {
        return Err(Error::InvalidInput(
            "group commit exceeds the transaction limit",
        ));
    }
    Ok(())
}

/// Full storage and runtime observability snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct OperationalStats {
    /// Verification result for the latest durable snapshot.
    pub snapshot: VerificationReport,
    /// Current physical length of the database file.
    pub file_bytes: u64,
    /// Bytes through the latest committed page boundary.
    pub committed_bytes: u64,
    /// Metadata and reachable live-page bytes.
    pub live_bytes: u64,
    /// Committed bytes no longer reachable from the latest root.
    pub retired_bytes: u64,
    /// Bytes written after the committed boundary by interrupted commits.
    pub uncommitted_tail_bytes: u64,
    /// Decoded-page cache counters.
    pub page_cache: PageCacheStats,
    /// End-to-end latency of successful commits in this process.
    ///
    /// One explicit or automatically coalesced group counts as one operation
    /// even though its transaction ID advances once per batch.
    pub commits: LatencyStats,
    /// Combined data and metadata sync latency in this process.
    pub syncs: LatencyStats,
    /// Pre-encoding work and append-boundary validation for successful WAL appends.
    pub wal_preflights: LatencyStats,
    /// Time spent encoding successful buffered WAL appends.
    pub wal_encodes: LatencyStats,
    /// Time spent issuing successful write-buffer writes in this process.
    pub wal_writes: LatencyStats,
    /// Time spent synchronizing successful write-buffer appends to storage.
    pub wal_syncs: LatencyStats,
    /// Work after a durable WAL append until buffered commit completion.
    pub wal_publications: LatencyStats,
    /// End-to-end lifetime of successfully published primary-tree checkpoints.
    pub checkpoints: LatencyStats,
    /// Primary-tree checkpoints whose preparation started in this process.
    pub checkpoint_starts: u64,
    /// Primary-tree checkpoints currently preparing or awaiting publication.
    pub checkpoints_in_progress: u64,
    /// Data-page and metadata bytes written by successful checkpoints.
    pub checkpoint_bytes: u64,
    /// Logical mutation bytes and database-file bytes written by successful
    /// commits in this process.
    pub writes: WriteStats,
    /// Corruption errors observed by reads and explicit verification.
    pub corruption_detections: u64,
    /// Automatic write-buffer checkpoints that could not be completed.
    ///
    /// The triggering commit remains durable in the write buffer.
    pub checkpoint_failures: u64,
}

impl Database {
    pub(crate) fn from_store(store: Arc<dyn StorageEngine>, head: Head) -> Self {
        Self {
            inner: Arc::new(DatabaseInner {
                state: RwLock::new(DatabaseState { head, store }),
                writer: Mutex::new(()),
                batch_commits: BatchCommitCoordinator::new(),
            }),
        }
    }

    fn snapshot_state(&self) -> Result<(Arc<dyn StorageEngine>, Head)> {
        let state = self
            .inner
            .state
            .read()
            .map_err(|_| Error::LockPoisoned("database state"))?;
        Ok((Arc::clone(&state.store), state.head))
    }

    /// Starts a read transaction pinned to the current committed tree root.
    pub fn read(&self) -> Result<ReadTransaction> {
        let state = self
            .inner
            .state
            .read()
            .map_err(|_| Error::LockPoisoned("database state"))?;
        Ok(ReadTransaction::new(Arc::clone(&state.store), state.head))
    }

    /// Starts the only writer associated with this database handle.
    ///
    /// The call waits until an earlier writer commits, aborts, or is dropped.
    pub fn write(&self) -> Result<WriteTransaction<'_>> {
        let writer_guard = self
            .inner
            .writer
            .lock()
            .map_err(|_| Error::LockPoisoned("writer"))?;
        let state = self
            .inner
            .state
            .read()
            .map_err(|_| Error::LockPoisoned("database state"))?;

        Ok(WriteTransaction::new(
            &self.inner.state,
            Arc::clone(&state.store),
            writer_guard,
            state.head,
        ))
    }

    /// Durably commits ordered batches with one write-ahead-log sync.
    ///
    /// Each batch receives its own consecutive transaction ID and observes
    /// mutations from earlier batches in the group. The final transaction ID
    /// is returned after every batch is durable. This operation requires a
    /// nonzero [`DatabaseOptions::write_buffer_bytes`] setting. An empty group
    /// is rejected; individual batches may be empty. The combined encoded WAL
    /// records are limited to 1 GiB.
    ///
    /// [`Error::CommitOutcomeUnknown`] means the WAL sync began but its result
    /// could not be confirmed. Refresh or reopen and inspect the transaction ID
    /// before retrying any batch.
    pub fn commit_group(&self, batches: Vec<WriteBatch>) -> Result<TransactionId> {
        if batches.is_empty() {
            return Err(Error::InvalidInput("group commit requires a batch"));
        }
        validate_group_limit(batches.len())?;
        let _writer_guard = self
            .inner
            .writer
            .lock()
            .map_err(|_| Error::LockPoisoned("writer"))?;
        let transaction_ids = self.commit_group_locked(batches.into_iter())?;
        transaction_ids
            .last()
            .copied()
            .ok_or(Error::InternalInvariant("group commit result is empty"))
    }

    /// Durably commits one blind-write batch, automatically sharing a WAL sync
    /// with unrelated concurrent callers when possible.
    ///
    /// The first queued caller proceeds immediately; there is no grouping
    /// delay or background thread. Calls that overlap an active commit are
    /// ordered by arrival and form the next durable group. Every caller waits
    /// for its own batch and receives its own consecutive transaction ID.
    /// This operation requires a nonzero
    /// [`DatabaseOptions::write_buffer_bytes`] setting.
    ///
    /// [`Error::CommitOutcomeUnknown`] applies to every member of a group whose
    /// WAL synchronization could not be confirmed. Refresh or reopen and
    /// inspect the durable transaction ID before retrying.
    pub fn commit_batch(&self, batch: WriteBatch) -> Result<TransactionId> {
        let waiting = self.inner.batch_commits.enqueue(batch);
        let mut is_leader = waiting.is_leader;
        loop {
            if is_leader {
                self.commit_next_automatic_group();
            }
            match waiting.receive() {
                WaitAction::Complete(result) => return result,
                WaitAction::Lead => is_leader = true,
            }
        }
    }

    fn commit_next_automatic_group(&self) {
        let _leadership = self.inner.batch_commits.leadership_guard();
        let writer_guard = self.inner.writer.lock();
        let mut pending = match self.inner.batch_commits.take_group() {
            Ok(pending) => pending,
            Err(error) => {
                self.inner.batch_commits.reject_pending(&error);
                return;
            }
        };
        let result = match &writer_guard {
            Ok(_guard) => {
                self.commit_group_locked(pending.iter_mut().map(|request| request.take_batch()))
            }
            Err(_) => Err(Error::LockPoisoned("writer")),
        };
        if matches!(
            &result,
            Err(Error::CommitOutcomeUnknown(_)
                | Error::PublicationOutcomeUnknown(_)
                | Error::InternalInvariant(_)
                | Error::LockPoisoned(_))
        ) {
            self.inner.batch_commits.fail_closed();
        }
        complete_group(pending, result);
    }

    fn commit_group_locked(
        &self,
        batches: impl ExactSizeIterator<Item = WriteBatch>,
    ) -> Result<Vec<TransactionId>> {
        let batch_count = batches.len();
        if batch_count == 0 {
            return Err(Error::InternalInvariant("automatic group is empty"));
        }
        validate_group_limit(batch_count)?;
        let (store, head) = {
            let state = self
                .inner
                .state
                .read()
                .map_err(|_| Error::LockPoisoned("database state"))?;
            (Arc::clone(&state.store), state.head)
        };
        let transaction_ids = consecutive_transaction_ids(head.transaction_id(), batch_count)?;
        let mut changes = Vec::new();
        changes
            .try_reserve_exact(batch_count)
            .map_err(|_| Error::FileTooLarge)?;
        changes.extend(batches.map(WriteBatch::into_changes));
        let expected = transaction_ids
            .last()
            .copied()
            .ok_or(Error::InternalInvariant("group commit result is empty"))?;
        let committed = store
            .commit_group(head, changes)?
            .require_transaction_id(expected)?;
        let mut state = self
            .inner
            .state
            .write()
            .map_err(|_| Error::LockPoisoned("database state"))?;
        if let Some(store) = committed.store {
            state.store = store;
        }
        state.head = committed.head;
        Ok(transaction_ids)
    }

    /// Returns statistics for the latest committed tree version.
    pub fn stats(&self) -> Result<DatabaseStats> {
        let state = self
            .inner
            .state
            .read()
            .map_err(|_| Error::LockPoisoned("database state"))?;
        Ok(DatabaseStats {
            transaction_id: state.head.transaction_id(),
            entries: state.head.entry_count(),
            last_page: state.head.last_page(),
        })
    }

    /// Validates every reachable page and structural invariant in the latest
    /// committed snapshot.
    ///
    /// This operation reads the complete live tree and every overflow value.
    /// It can therefore be expensive for large databases. The database handle
    /// state lock is released after capturing the immutable snapshot.
    pub fn verify(&self) -> Result<VerificationReport> {
        let (store, head) = self.snapshot_state()?;
        store.verify(head)
    }

    /// Returns current decoded-page cache counters.
    pub fn page_cache_stats(&self) -> Result<PageCacheStats> {
        let state = self
            .inner
            .state
            .read()
            .map_err(|_| Error::LockPoisoned("database state"))?;
        state.store.page_cache_stats()
    }

    /// Returns lightweight primary-tree checkpoint counters.
    ///
    /// Unlike [`Self::operational_stats`], this method does not read or verify
    /// database pages and can be sampled while measuring a workload.
    pub fn checkpoint_stats(&self) -> Result<CheckpointStats> {
        let (store, _) = self.snapshot_state()?;
        store.checkpoint_stats()
    }

    /// Creates a verified snapshot backup without overwriting `target`.
    ///
    /// Commits are briefly blocked while the immutable source snapshot is
    /// captured, then continue while its committed file range is copied. The
    /// final path is published only after the copy is synced and fully
    /// validated. [`Error::PublicationOutcomeUnknown`] means the target may
    /// already contain the complete snapshot and must be inspected before a
    /// retry.
    pub fn backup(&self, target: impl AsRef<Path>) -> Result<BackupReport> {
        let (store, _) = self.snapshot_state()?;
        store.backup(target.as_ref())
    }

    /// Rebuilds the latest durable snapshot into a verified compact file.
    ///
    /// The destination is never overwritten. The source database remains open
    /// and usable, including for concurrent commits, throughout the rebuild.
    /// [`Error::PublicationOutcomeUnknown`] means the target may already exist
    /// with the complete compact snapshot.
    pub fn compact_to(&self, target: impl AsRef<Path>) -> Result<CompactionReport> {
        let (store, _) = self.snapshot_state()?;
        store.compact_to(target.as_ref())
    }

    /// Consolidates durable buffered commits into one ordered snapshot record.
    ///
    /// Commits are durable before this call. On Unix, flushing atomically
    /// replaces the transaction chain with its latest mutation per key,
    /// bounding recovery work without rewriting the primary tree. A snapshot
    /// too large for one record, and platforms without atomic replacement,
    /// fall back to a primary-tree checkpoint. Existing read transactions
    /// remain pinned to their original snapshot. After any flush error,
    /// refresh this handle before issuing another write: asynchronous
    /// preparation or publication may already have advanced durable state.
    /// [`Error::PublicationOutcomeUnknown`] specifically means publication
    /// may have completed even though its acknowledgement failed.
    pub fn flush(&self) -> Result<()> {
        let _writer_guard = self
            .inner
            .writer
            .lock()
            .map_err(|_| Error::LockPoisoned("writer"))?;
        let (store, head) = {
            let state = self
                .inner
                .state
                .read()
                .map_err(|_| Error::LockPoisoned("database state"))?;
            (Arc::clone(&state.store), state.head)
        };
        let flushed = store.flush(head)?.require_logical_identity(head)?;
        let mut state = self
            .inner
            .state
            .write()
            .map_err(|_| Error::LockPoisoned("database state"))?;
        if let Some(store) = flushed.store {
            state.store = store;
        }
        state.head = flushed.head;
        Ok(())
    }

    /// Atomically compacts the active database and keeps this handle usable.
    ///
    /// Existing read transactions continue through the old file generation.
    /// New transactions use the verified compact generation. The operation is
    /// serialized with writers associated with this handle and fails with
    /// [`Error::WriteConflict`] if another handle publishes first. After
    /// [`Error::PublicationOutcomeUnknown`], refresh this handle before
    /// issuing another write.
    pub fn compact(&self) -> Result<CompactionReport> {
        let _writer_guard = self
            .inner
            .writer
            .lock()
            .map_err(|_| Error::LockPoisoned("writer"))?;
        let (store, head) = {
            let state = self
                .inner
                .state
                .read()
                .map_err(|_| Error::LockPoisoned("database state"))?;
            (Arc::clone(&state.store), state.head)
        };
        let compacted = store
            .compact_active(head)?
            .require_consistent_report(head)?;
        let mut state = self
            .inner
            .state
            .write()
            .map_err(|_| Error::LockPoisoned("database state"))?;
        state.store = compacted.store;
        state.head = compacted.head;
        Ok(compacted.report)
    }

    /// Refreshes this handle to the latest durable generation and snapshot.
    ///
    /// Existing read transactions keep their original snapshot. Use this after
    /// another process or separately opened handle commits or compacts the
    /// database. The returned identifier is the snapshot selected on reopen.
    /// Refresh never moves a live handle to a lower transaction identifier;
    /// an older or incomplete external generation returns
    /// [`Error::WriteConflict`] and leaves the current snapshot installed. An
    /// alternative head or replacement generation at the same transaction
    /// identifier is accepted only when full verification finds identical
    /// logical contents.
    pub fn refresh(&self) -> Result<TransactionId> {
        let _writer_guard = self
            .inner
            .writer
            .lock()
            .map_err(|_| Error::LockPoisoned("writer"))?;
        let (store, current_head) = {
            let state = self
                .inner
                .state
                .read()
                .map_err(|_| Error::LockPoisoned("database state"))?;
            (Arc::clone(&state.store), state.head)
        };
        let refreshed = store.refresh()?;
        if refreshed.head.transaction_id() < current_head.transaction_id() {
            return Err(Error::WriteConflict);
        }
        if refreshed.head.transaction_id() == current_head.transaction_id()
            && (refreshed.store.is_some() || refreshed.head != current_head)
        {
            let refreshed_store = refreshed.store.as_ref().unwrap_or(&store);
            let current = store.verify(current_head)?;
            let candidate = refreshed_store.verify(refreshed.head)?;
            if current.entries != candidate.entries
                || current.logical_hash != candidate.logical_hash
            {
                return Err(Error::WriteConflict);
            }
        }
        let transaction_id = refreshed.head.transaction_id();
        let mut state = self
            .inner
            .state
            .write()
            .map_err(|_| Error::LockPoisoned("database state"))?;
        if let Some(store) = refreshed.store {
            state.store = store;
        }
        state.head = refreshed.head;
        drop(state);
        self.inner.batch_commits.reset_after_refresh();
        Ok(transaction_id)
    }

    /// Performs a full verification and returns storage amplification and
    /// process-local runtime counters for the latest durable snapshot.
    pub fn operational_stats(&self) -> Result<OperationalStats> {
        let (store, _) = self.snapshot_state()?;
        store.operational_stats()
    }
}

#[cfg(test)]
mod tests;
