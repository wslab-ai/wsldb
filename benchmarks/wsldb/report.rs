use std::error::Error;
use std::fs;

use super::{BenchmarkDatabase, Config};
use wsldb::{Database, OperationalStats};

pub(super) fn database(
    database: &Database,
    database_file: &BenchmarkDatabase,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    let stats = database.page_cache_stats()?;
    eprintln!(
        "entries={} batch_size={} value_bytes={} cache_pages={} write_buffer_bytes={} cache_hits={} cache_misses={} resident_pages={} main_file_bytes={}",
        config.entries,
        config.batch_size,
        config.value_bytes,
        stats.capacity,
        config.write_buffer_bytes,
        stats.hits,
        stats.misses,
        stats.resident_pages,
        fs::metadata(database_file.path())?.len(),
    );
    Ok(())
}

pub(super) fn load_database(
    database: &Database,
    database_file: &BenchmarkDatabase,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    self::database(database, database_file, config)?;
    let stats = database.operational_stats()?;
    eprintln!(
        "load_complete entries={} logical_hash={}",
        stats.snapshot.entries,
        encode_hex(&stats.snapshot.logical_hash),
    );
    operational(&stats, database_file)
}

pub(super) fn operational(
    stats: &OperationalStats,
    database_file: &BenchmarkDatabase,
) -> Result<(), Box<dyn Error>> {
    let amplification = if stats.writes.logical_mutation_bytes == 0 {
        0.0
    } else {
        stats.writes.database_bytes() as f64 / stats.writes.logical_mutation_bytes as f64
    };
    eprintln!(
        "commit_operations={} commit_total_ns={} commit_max_ns={} sync_operations={} sync_total_ns={} sync_max_ns={} wal_preflight_operations={} wal_preflight_total_ns={} wal_preflight_max_ns={} wal_encode_operations={} wal_encode_total_ns={} wal_encode_max_ns={} wal_write_operations={} wal_write_total_ns={} wal_write_max_ns={} wal_sync_operations={} wal_sync_total_ns={} wal_sync_max_ns={} wal_sync_p50_ns={} wal_sync_p95_ns={} wal_sync_p99_ns={} wal_sync_p999_ns={} wal_publication_operations={} wal_publication_total_ns={} wal_publication_max_ns={} checkpoint_starts={} checkpoints_in_progress={} checkpoint_operations={} checkpoint_total_ns={} checkpoint_max_ns={} checkpoint_p99_ns={} checkpoint_bytes={}",
        stats.commits.operations,
        stats.commits.total_nanos,
        stats.commits.max_nanos,
        stats.syncs.operations,
        stats.syncs.total_nanos,
        stats.syncs.max_nanos,
        stats.wal_preflights.operations,
        stats.wal_preflights.total_nanos,
        stats.wal_preflights.max_nanos,
        stats.wal_encodes.operations,
        stats.wal_encodes.total_nanos,
        stats.wal_encodes.max_nanos,
        stats.wal_writes.operations,
        stats.wal_writes.total_nanos,
        stats.wal_writes.max_nanos,
        stats.wal_syncs.operations,
        stats.wal_syncs.total_nanos,
        stats.wal_syncs.max_nanos,
        stats.wal_syncs.p50_nanos,
        stats.wal_syncs.p95_nanos,
        stats.wal_syncs.p99_nanos,
        stats.wal_syncs.p999_nanos,
        stats.wal_publications.operations,
        stats.wal_publications.total_nanos,
        stats.wal_publications.max_nanos,
        stats.checkpoint_starts,
        stats.checkpoints_in_progress,
        stats.checkpoints.operations,
        stats.checkpoints.total_nanos,
        stats.checkpoints.max_nanos,
        stats.checkpoints.p99_nanos,
        stats.checkpoint_bytes,
    );
    eprintln!(
        "logical_mutation_bytes={} data_page_bytes={} leaf_page_bytes={} branch_page_bytes={} overflow_page_bytes={} write_buffer_bytes={} metadata_bytes={} database_write_bytes={} database_write_amplification={amplification:.3} storage_bytes={} wal_file_bytes={} committed_bytes={} live_bytes={} retired_bytes={} uncommitted_tail_bytes={} checkpoint_failures={} logical_hash={}",
        stats.writes.logical_mutation_bytes,
        stats.writes.data_page_bytes,
        stats.writes.leaf_page_bytes,
        stats.writes.branch_page_bytes,
        stats.writes.overflow_page_bytes,
        stats.writes.write_buffer_bytes,
        stats.writes.metadata_bytes,
        stats.writes.database_bytes(),
        stats.file_bytes,
        stats
            .file_bytes
            .saturating_sub(fs::metadata(database_file.path())?.len()),
        stats.committed_bytes,
        stats.live_bytes,
        stats.retired_bytes,
        stats.uncommitted_tail_bytes,
        stats.checkpoint_failures,
        encode_hex(&stats.snapshot.logical_hash),
    );
    Ok(())
}

pub(super) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(*byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(*byte & 0x0f)]));
    }
    encoded
}
