mod support;

use std::collections::BTreeMap;
use std::fs;
#[cfg(feature = "crash-injection")]
use std::process::{Command, Stdio};

use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error};

const PAGE_SIZE: u64 = 4096;

#[test]
fn backup_is_snapshot_consistent_verified_and_non_overwriting() {
    let source = TestFile::new("backup-source");
    let backup = TestFile::new("backup-target");
    let database = Database::open(source.path()).unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..300_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.put(b"large", &vec![17; 25_000]).unwrap();
    writer.commit().unwrap();
    let expected: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    let source_report = database.verify().unwrap();

    let report = database.backup(backup.path()).unwrap();
    assert_eq!(report.snapshot.entries, 301);
    assert!(report.snapshot.overflow_pages > 0);
    assert_eq!(report.snapshot.logical_hash, source_report.logical_hash);
    assert_eq!(
        report.bytes_copied,
        fs::metadata(backup.path()).unwrap().len()
    );
    let restored = Database::open(backup.path()).unwrap();
    assert_eq!(restored.verify().unwrap(), report.snapshot);
    assert_eq!(
        restored
            .read()
            .unwrap()
            .iter()
            .unwrap()
            .collect::<BTreeMap<_, _>>(),
        expected
    );

    let occupied = TestFile::new("backup-occupied");
    fs::write(occupied.path(), b"do-not-overwrite").unwrap();
    assert!(matches!(
        database.backup(occupied.path()),
        Err(Error::Io(_))
    ));
    assert_eq!(fs::read(occupied.path()).unwrap(), b"do-not-overwrite");
}

#[test]
fn compaction_rebuilds_live_data_and_discards_retired_pages() {
    let source = TestFile::new("compact-source");
    let compacted = TestFile::new("compact-target");
    let database = Database::open(source.path()).unwrap();
    let mut initial = database.write().unwrap();
    for index in 0..1000_u32 {
        initial
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("initial-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    initial.commit().unwrap();

    for round in 0..8_u32 {
        let mut writer = database.write().unwrap();
        for index in (round..1000).step_by(8) {
            writer
                .put(
                    format!("key-{index:04}").as_bytes(),
                    format!("round-{round}-{index:04}").as_bytes(),
                )
                .unwrap();
        }
        writer.commit().unwrap();
    }
    let mut final_writer = database.write().unwrap();
    for index in 0..300_u32 {
        final_writer
            .delete(format!("key-{index:04}").as_bytes())
            .unwrap();
    }
    final_writer.put(b"large", &vec![23; 25_000]).unwrap();
    final_writer.commit().unwrap();
    let expected: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    let source_hash = database.verify().unwrap().logical_hash;

    let report = database.compact_to(compacted.path()).unwrap();
    assert_eq!(
        report.source_bytes,
        fs::metadata(source.path()).unwrap().len()
    );
    assert_eq!(
        report.compacted_bytes,
        fs::metadata(compacted.path()).unwrap().len()
    );
    assert!(report.compacted_bytes < report.source_bytes);
    assert!(report.publication_pause.is_zero());
    assert_eq!(report.compacted_snapshot.entries, expected.len() as u64);
    assert_eq!(report.compacted_snapshot.logical_hash, source_hash);

    let reopened = Database::open(compacted.path()).unwrap();
    assert_eq!(reopened.verify().unwrap(), report.compacted_snapshot);
    assert_eq!(
        reopened
            .read()
            .unwrap()
            .iter()
            .unwrap()
            .collect::<BTreeMap<_, _>>(),
        expected
    );
}

#[test]
fn atomic_compaction_preserves_old_readers_and_rejects_stale_writers() {
    let file = TestFile::new("atomic-compact");
    let database = Database::open(file.path()).unwrap();
    let mut initial = database.write().unwrap();
    for index in 0..500_u32 {
        initial
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    initial.commit().unwrap();
    let mut churn = database.write().unwrap();
    for index in (0..500_u32).step_by(5) {
        churn
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("updated-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    churn.commit().unwrap();
    let old_reader = database.read().unwrap();
    let old_transaction = old_reader.transaction_id();
    let mut stale_writer = database.write().unwrap();
    stale_writer.put(b"stale", b"must-not-commit").unwrap();
    let before = fs::metadata(file.path()).unwrap().len();

    let report = Database::compact_file(file.path()).unwrap();

    assert_eq!(report.source_transaction_id, old_transaction);
    assert_eq!(
        report.compacted_snapshot.transaction_id.value(),
        old_transaction.value() + 1
    );
    assert!(report.compacted_bytes < before);
    assert_eq!(
        old_reader.get(b"key-0042").unwrap(),
        Some(b"value-0042".to_vec())
    );
    assert!(matches!(stale_writer.commit(), Err(Error::WriteConflict)));

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap(), report.compacted_snapshot);
    assert_eq!(reopened.read().unwrap().get(b"stale").unwrap(), None);
}

#[test]
fn active_compaction_keeps_the_database_handle_writable() {
    let file = TestFile::new("active-compaction");
    let database = Database::open(file.path()).unwrap();
    let mut initial = database.write().unwrap();
    for index in 0..2_000_u32 {
        initial
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("initial-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    initial.commit().unwrap();
    for round in 0..4_u32 {
        let mut writer = database.write().unwrap();
        for index in 0..2_000_u32 {
            writer
                .put(
                    format!("key-{index:04}").as_bytes(),
                    format!("round-{round}-{index:04}").as_bytes(),
                )
                .unwrap();
        }
        writer.commit().unwrap();
    }

    let old_reader = database.read().unwrap();
    let stale_database = Database::open(file.path()).unwrap();
    let mut stale_writer = stale_database.write().unwrap();
    stale_writer.put(b"stale", b"must-not-commit").unwrap();
    let before = fs::metadata(file.path()).unwrap().len();

    let report = database.compact().unwrap();
    assert!(report.compacted_bytes < before);
    assert_eq!(
        database.stats().unwrap().transaction_id,
        report.compacted_snapshot.transaction_id
    );
    assert_eq!(
        old_reader.get(b"key-0042").unwrap(),
        Some(b"round-3-0042".to_vec())
    );
    assert!(matches!(stale_writer.commit(), Err(Error::WriteConflict)));
    assert_eq!(
        stale_database.refresh().unwrap(),
        report.compacted_snapshot.transaction_id
    );
    assert_eq!(
        stale_database.read().unwrap().get(b"key-0042").unwrap(),
        Some(b"round-3-0042".to_vec())
    );

    let mut after = database.write().unwrap();
    after.put(b"after", b"still-writable").unwrap();
    after.commit().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"after").unwrap(),
        Some(b"still-writable".to_vec())
    );
    assert_eq!(database.verify().unwrap().entries, 2_001);
    assert_eq!(database.operational_stats().unwrap().commits.operations, 6);
    assert_eq!(
        stale_database.refresh().unwrap(),
        database.stats().unwrap().transaction_id
    );
    assert_eq!(
        stale_database.read().unwrap().get(b"after").unwrap(),
        Some(b"still-writable".to_vec())
    );

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(
        reopened.read().unwrap().get(b"after").unwrap(),
        Some(b"still-writable".to_vec())
    );
}

#[test]
fn buffered_active_compaction_preserves_old_readers_and_future_writes() {
    let file = TestFile::new("buffered-active-compact");
    let database = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let mut initial = database.write().unwrap();
    for index in 0..500_u32 {
        initial
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    initial.commit().unwrap();
    let old_reader = database.read().unwrap();
    let old_transaction = old_reader.transaction_id();

    let report = database.compact().unwrap();
    assert_eq!(report.source_transaction_id, old_transaction);
    assert_eq!(report.compacted_snapshot.transaction_id, old_transaction);
    assert_eq!(
        old_reader.get(b"key-0042").unwrap(),
        Some(b"value-0042".to_vec())
    );
    let mut later = database.write().unwrap();
    later.put(b"later", b"value").unwrap();
    later.commit().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"later").unwrap(),
        Some(b"value".to_vec())
    );
    drop(old_reader);
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 501);
    assert_eq!(
        reopened.read().unwrap().get(b"later").unwrap(),
        Some(b"value".to_vec())
    );
}

#[test]
fn buffered_flush_preserves_the_warm_primary_page_cache() {
    let file = TestFile::new("buffered-flush-cache");
    {
        let database = Database::open(file.path()).unwrap();
        let mut writer = database.write().unwrap();
        for index in 0..500_u32 {
            writer
                .put(
                    format!("key-{index:04}").as_bytes(),
                    format!("value-{index:04}").as_bytes(),
                )
                .unwrap();
        }
        writer.commit().unwrap();
    }

    let database = Database::open_with_options(
        file.path(),
        DatabaseOptions::new()
            .page_cache_capacity(128)
            .write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    assert_eq!(
        database.read().unwrap().get(b"key-0250").unwrap(),
        Some(b"value-0250".to_vec())
    );
    let warm = database.page_cache_stats().unwrap();
    assert!(warm.misses > 0);

    let mut writer = database.write().unwrap();
    writer.put(b"key-0001", b"updated").unwrap();
    writer.commit().unwrap();
    let before_flush = database.page_cache_stats().unwrap();
    database.flush().unwrap();
    assert_eq!(database.page_cache_stats().unwrap(), before_flush);

    assert_eq!(
        database.read().unwrap().get(b"key-0250").unwrap(),
        Some(b"value-0250".to_vec())
    );
    let reused = database.page_cache_stats().unwrap();
    assert!(reused.hits > before_flush.hits);
    assert_eq!(reused.misses, before_flush.misses);
}

#[cfg(feature = "crash-injection")]
#[test]
fn interrupted_atomic_compaction_keeps_a_complete_generation() {
    let baseline = TestFile::new("compact-crash-baseline");
    let database = Database::open(baseline.path()).unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..1_000_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.commit().unwrap();
    let expected = database.verify().unwrap().logical_hash;
    drop(database);

    for stage in ["compaction_ready", "compaction_rename"] {
        let target = TestFile::new("compact-crash-target");
        fs::copy(baseline.path(), target.path()).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("compact_crash_worker")
            .arg("--nocapture")
            .env("WSLDB_COMPACT_CRASH_PATH", target.path())
            .env("WSLDB_FAILPOINT", stage)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success());
        assert_eq!(
            Database::open(target.path())
                .unwrap()
                .verify()
                .unwrap()
                .logical_hash,
            expected
        );
    }
}

#[cfg(feature = "crash-injection")]
#[test]
fn compact_crash_worker() {
    let Some(path) = std::env::var_os("WSLDB_COMPACT_CRASH_PATH") else {
        return;
    };
    Database::compact_file(path).unwrap();
}

#[test]
fn operational_stats_report_space_and_commit_metrics() {
    let file = TestFile::new("operational-stats");
    let database = Database::open(file.path()).unwrap();
    let mut first = database.write().unwrap();
    for index in 0..200_u32 {
        first
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("first-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    first.commit().unwrap();
    let mut second = database.write().unwrap();
    for index in 0..100_u32 {
        second
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("second-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    second.commit().unwrap();

    let stats = database.operational_stats().unwrap();
    assert_eq!(stats.snapshot.entries, 200);
    assert_eq!(stats.file_bytes, fs::metadata(file.path()).unwrap().len());
    assert!(stats.live_bytes < stats.committed_bytes);
    assert_eq!(
        stats.retired_bytes,
        stats.committed_bytes - stats.live_bytes
    );
    assert_eq!(stats.uncommitted_tail_bytes, 0);
    assert_eq!(stats.commits.operations, 2);
    assert_eq!(stats.syncs.operations, 2);
    assert_eq!(stats.wal_preflights.operations, 0);
    assert_eq!(stats.wal_encodes.operations, 0);
    assert_eq!(stats.wal_writes.operations, 0);
    assert_eq!(stats.wal_syncs.operations, 0);
    assert_eq!(stats.wal_publications.operations, 0);
    assert_eq!(stats.checkpoint_starts, 0);
    assert_eq!(stats.checkpoints_in_progress, 0);
    assert_eq!(stats.checkpoints.operations, 0);
    assert_eq!(stats.checkpoint_bytes, 0);
    assert!(stats.commits.total_nanos >= stats.commits.last_nanos);
    assert!(stats.syncs.max_nanos > 0);
    assert!(stats.commits.p50_nanos > 0);
    assert!(stats.commits.p50_nanos <= stats.commits.p95_nanos);
    assert!(stats.commits.p95_nanos <= stats.commits.p99_nanos);
    assert!(stats.commits.p99_nanos <= stats.commits.p999_nanos);
    assert!(stats.commits.p999_nanos >= stats.commits.max_nanos);
    assert_eq!(stats.wal_syncs.p99_nanos, 0);
    assert_eq!(stats.writes.logical_mutation_bytes, 5_500);
    assert!(stats.writes.data_page_bytes > 0);
    assert_eq!(stats.writes.data_page_bytes % PAGE_SIZE, 0);
    assert!(stats.writes.leaf_page_bytes > 0);
    assert!(stats.writes.branch_page_bytes > 0);
    assert_eq!(stats.writes.overflow_page_bytes, 0);
    assert_eq!(stats.writes.write_buffer_bytes, 0);
    assert_eq!(
        stats.writes.data_page_bytes,
        stats.writes.leaf_page_bytes
            + stats.writes.branch_page_bytes
            + stats.writes.overflow_page_bytes
    );
    assert_eq!(stats.writes.metadata_bytes, 2 * PAGE_SIZE);
    assert_eq!(
        stats.writes.database_bytes(),
        stats.writes.data_page_bytes
            + stats.writes.write_buffer_bytes
            + stats.writes.metadata_bytes
    );
    assert_eq!(stats.corruption_detections, 0);
    assert_eq!(stats.checkpoint_failures, 0);
}

#[test]
fn operational_stats_attribute_overflow_page_writes() {
    let file = TestFile::new("operational-stats-overflow");
    let database = Database::open(file.path()).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"large", &vec![31; 10_000]).unwrap();
    writer.commit().unwrap();

    let writes = database.operational_stats().unwrap().writes;
    assert!(writes.leaf_page_bytes > 0);
    assert_eq!(writes.branch_page_bytes, 0);
    assert!(writes.overflow_page_bytes > 0);
    assert_eq!(writes.write_buffer_bytes, 0);
    assert_eq!(
        writes.data_page_bytes,
        writes.leaf_page_bytes + writes.branch_page_bytes + writes.overflow_page_bytes
    );
}
