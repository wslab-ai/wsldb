use std::error::Error as StdError;
use std::fmt::Debug;
use std::iter::FusedIterator;
use std::path::Path;

use wsldb::{
    BackupReport, CheckpointStats, CompactionReport, Database, DatabaseOptions, DatabaseStats,
    EntryIter, Error, LatencyStats, MAX_GROUP_TRANSACTIONS, MAX_KEY_BYTES, MAX_VALUE_BYTES,
    OperationalStats, PageCacheStats, ReadTransaction, Result, TransactionId, VerificationReport,
    WriteBatch, WriteStats, WriteTransaction,
};

#[test]
fn public_types_keep_their_thread_safety_contracts() {
    assert_send_sync::<Database>();
    assert_send_sync::<DatabaseOptions>();
    assert_send_sync::<DatabaseStats>();
    assert_send_sync::<ReadTransaction>();
    assert_send_sync::<EntryIter>();
    assert_send_sync::<WriteBatch>();
    assert_send_sync::<TransactionId>();
    assert_send_sync::<VerificationReport>();
    assert_send_sync::<BackupReport>();
    assert_send_sync::<CompactionReport>();
    assert_send_sync::<PageCacheStats>();
    assert_send_sync::<LatencyStats>();
    assert_send_sync::<CheckpointStats>();
    assert_send_sync::<WriteStats>();
    assert_send_sync::<OperationalStats>();
    assert_send_sync::<Error>();
}

#[test]
fn public_types_keep_their_trait_contracts() {
    assert_clone::<Database>();
    assert_clone_debug_default_eq::<DatabaseOptions>();
    assert_copy_debug_eq::<DatabaseStats>();
    assert_copy_debug_eq::<VerificationReport>();
    assert_copy_debug_eq::<BackupReport>();
    assert_copy_debug_eq::<CompactionReport>();
    assert_copy_debug_eq::<PageCacheStats>();
    assert_copy_debug_eq::<LatencyStats>();
    assert_copy_debug_eq::<CheckpointStats>();
    assert_copy_debug_eq::<WriteStats>();
    assert_copy_debug_eq::<OperationalStats>();
    assert_clone_default::<WriteBatch>();
    assert_copy_debug_default_ord::<TransactionId>();
    assert_entry_iterator::<EntryIter>();
    assert_debug::<Error>();
}

#[test]
fn complete_public_surface_compiles_as_an_external_consumer() {
    let _: usize = MAX_GROUP_TRANSACTIONS;
    let _: usize = MAX_KEY_BYTES;
    let _: usize = MAX_VALUE_BYTES;
    assert_eq!(MAX_KEY_BYTES, 1024);
    assert_eq!(MAX_VALUE_BYTES, 64 * 1024 * 1024);
    let _: fn(&Path, &Path, &Path) -> Result<()> = exercise_public_surface;
    let _: for<'database> fn(WriteTransaction<'database>) = accept_write_transaction;
    let _: fn(Error) = inspect_error;
}

fn exercise_public_surface(path: &Path, backup: &Path, compacted: &Path) -> Result<()> {
    let options = DatabaseOptions::default()
        .page_cache_capacity(128)
        .write_buffer_bytes(1_048_576)
        .verify_on_open(true);
    let database = Database::open_with_options(path, options.clone())?;
    let _clone = database.clone();

    let initial: DatabaseStats = database.stats()?;
    let _ = (
        initial.transaction_id.value(),
        initial.entries,
        initial.last_page,
    );

    let reader: ReadTransaction = database.read()?;
    let _ = (
        reader.transaction_id(),
        reader.len(),
        reader.is_empty(),
        reader.get(b"key")?,
    );
    inspect_entries(reader.iter()?);
    inspect_entries(reader.range(b"a", Some(b"z"), 10)?);
    inspect_entries(reader.range_after(Some(b"a"), Some(b"z"), 10)?);

    let mut aborted: WriteTransaction<'_> = database.write()?;
    aborted.put(b"key", b"value")?;
    let _ = aborted.get(b"key")?;
    let _ = aborted.delete(b"key")?;
    aborted.abort();

    let mut committed = database.write()?;
    committed.put(b"key", b"value")?;
    let _transaction: TransactionId = committed.commit()?;

    let mut batch = WriteBatch::default();
    let _ = (batch.len(), batch.is_empty());
    batch.put(b"group", b"value")?;
    batch.delete(b"missing")?;
    let _transaction = database.commit_group(vec![batch])?;
    let mut automatic = WriteBatch::new();
    automatic.put(b"automatic", b"value")?;
    let _transaction = database.commit_batch(automatic)?;

    inspect_verification(database.verify()?);
    inspect_page_cache(database.page_cache_stats()?);
    inspect_checkpoints(database.checkpoint_stats()?);
    inspect_operational(database.operational_stats()?);
    inspect_backup(database.backup(backup)?);
    inspect_compaction(database.compact_to(compacted)?);
    database.flush()?;
    inspect_compaction(database.compact()?);
    let _transaction = database.refresh()?;
    inspect_compaction(Database::compact_file(path)?);
    let _reopened = Database::open(path)?;
    Ok(())
}

fn inspect_entries(mut entries: EntryIter) {
    let _ = entries.len();
    let _ = entries.next();
}

fn inspect_verification(report: VerificationReport) {
    let _ = (
        report.transaction_id,
        report.entries,
        report.tree_pages,
        report.overflow_pages,
        report.tree_depth,
        report.logical_hash,
    );
}

fn inspect_backup(report: BackupReport) {
    inspect_verification(report.snapshot);
    let _ = report.bytes_copied;
}

fn inspect_compaction(report: CompactionReport) {
    let _ = (
        report.source_transaction_id,
        report.source_bytes,
        report.compacted_bytes,
        report.publication_pause,
    );
    inspect_verification(report.compacted_snapshot);
}

fn inspect_page_cache(stats: PageCacheStats) {
    let _ = (
        stats.capacity,
        stats.resident_pages,
        stats.hits,
        stats.misses,
    );
}

fn inspect_latency(stats: LatencyStats) {
    let _ = (
        stats.operations,
        stats.total_nanos,
        stats.last_nanos,
        stats.max_nanos,
        stats.p50_nanos,
        stats.p95_nanos,
        stats.p99_nanos,
        stats.p999_nanos,
    );
}

fn inspect_checkpoints(stats: CheckpointStats) {
    inspect_latency(stats.successful);
    let _ = (
        stats.started,
        stats.in_progress,
        stats.failures,
        stats.database_bytes,
    );
}

fn inspect_writes(stats: WriteStats) {
    let _ = (
        stats.logical_mutation_bytes,
        stats.data_page_bytes,
        stats.leaf_page_bytes,
        stats.branch_page_bytes,
        stats.overflow_page_bytes,
        stats.write_buffer_bytes,
        stats.metadata_bytes,
        stats.database_bytes(),
    );
}

fn inspect_operational(stats: OperationalStats) {
    inspect_verification(stats.snapshot);
    inspect_page_cache(stats.page_cache);
    inspect_latency(stats.commits);
    inspect_latency(stats.syncs);
    inspect_latency(stats.wal_preflights);
    inspect_latency(stats.wal_encodes);
    inspect_latency(stats.wal_writes);
    inspect_latency(stats.wal_syncs);
    inspect_latency(stats.wal_publications);
    inspect_latency(stats.checkpoints);
    inspect_writes(stats.writes);
    let _ = (
        stats.file_bytes,
        stats.committed_bytes,
        stats.live_bytes,
        stats.retired_bytes,
        stats.uncommitted_tail_bytes,
        stats.checkpoint_starts,
        stats.checkpoints_in_progress,
        stats.checkpoint_bytes,
        stats.corruption_detections,
        stats.checkpoint_failures,
    );
}

fn inspect_error(error: Error) {
    let _ = error.to_string();
    let _ = error.source();
    match error {
        Error::Io(_)
        | Error::CommitOutcomeUnknown(_)
        | Error::PublicationOutcomeUnknown(_)
        | Error::Corrupt(_)
        | Error::InvalidInput(_)
        | Error::WriteConflict
        | Error::TransactionIdExhausted
        | Error::TreeDepthExceeded
        | Error::InternalInvariant(_)
        | Error::LockPoisoned(_)
        | Error::FileTooLarge => {}
        _ => {}
    }
}

fn accept_write_transaction(_transaction: WriteTransaction<'_>) {}

fn assert_send_sync<T: Send + Sync>() {}

fn assert_clone<T: Clone>() {}

fn assert_clone_debug_default_eq<T: Clone + Debug + Default + Eq>() {}

fn assert_copy_debug_eq<T: Copy + Debug + Eq>() {}

fn assert_clone_default<T: Clone + Default>() {}

fn assert_copy_debug_default_ord<T: Copy + Debug + Default + Ord>() {}

fn assert_entry_iterator<T>()
where
    T: Iterator<Item = (Vec<u8>, Vec<u8>)> + ExactSizeIterator + FusedIterator,
{
}

fn assert_debug<T: Debug>() {}
