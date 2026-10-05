mod support;

use std::collections::BTreeMap;
#[cfg(feature = "crash-injection")]
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error};

#[test]
fn buffered_commits_preserve_snapshots_and_recover_without_options() {
    let file = TestFile::new("buffered-recovery");
    let database = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let empty = database.read().unwrap();
    let mut first = database.write().unwrap();
    first.put(b"alpha", b"one").unwrap();
    first.put(b"beta", b"two").unwrap();
    first.commit().unwrap();
    let first_snapshot = database.read().unwrap();
    let mut second = database.write().unwrap();
    second.put(b"alpha", b"updated").unwrap();
    second.delete(b"beta").unwrap();
    second.put(b"gamma", b"three").unwrap();
    second.commit().unwrap();

    assert_eq!(empty.get(b"alpha").unwrap(), None);
    assert_eq!(first_snapshot.get(b"alpha").unwrap(), Some(b"one".to_vec()));
    assert_eq!(first_snapshot.get(b"beta").unwrap(), Some(b"two".to_vec()));
    assert_eq!(database.read().unwrap().get(b"beta").unwrap(), None);
    assert_eq!(database.verify().unwrap().entries, 2);
    drop(first_snapshot);
    drop(empty);
    drop(database);

    let reopened =
        Database::open_with_options(file.path(), DatabaseOptions::new().verify_on_open(true))
            .unwrap();
    let entries = reopened
        .read()
        .unwrap()
        .iter()
        .unwrap()
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        entries,
        BTreeMap::from([
            (b"alpha".to_vec(), b"updated".to_vec()),
            (b"gamma".to_vec(), b"three".to_vec()),
        ])
    );
    assert_eq!(reopened.verify().unwrap().transaction_id.value(), 2);
}

#[test]
fn automatic_buffer_checkpoints_preserve_old_readers() {
    let file = TestFile::new("buffer-checkpoint");
    let database =
        Database::open_with_options(file.path(), DatabaseOptions::new().write_buffer_bytes(1))
            .unwrap();
    let empty = database.read().unwrap();
    let mut first = database.write().unwrap();
    for index in 0..300_u32 {
        first
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    first.commit().unwrap();
    let before_update = database.read().unwrap();
    let mut update = database.write().unwrap();
    update.put(b"key-0042", b"updated").unwrap();
    update.delete(b"key-0043").unwrap();
    update.put(b"key-0300", b"inserted").unwrap();
    update.commit().unwrap();
    let mut after_carried_filter = database.write().unwrap();
    after_carried_filter
        .put(b"key-0300", b"updated again")
        .unwrap();
    after_carried_filter.delete(b"key-0044").unwrap();
    after_carried_filter.put(b"key-0301", b"inserted").unwrap();
    after_carried_filter.commit().unwrap();
    database.flush().unwrap();

    assert_eq!(empty.get(b"key-0042").unwrap(), None);
    assert_eq!(
        before_update.get(b"key-0042").unwrap(),
        Some(b"value-0042".to_vec())
    );
    assert_eq!(
        database.read().unwrap().get(b"key-0042").unwrap(),
        Some(b"updated".to_vec())
    );
    assert_eq!(database.read().unwrap().get(b"key-0043").unwrap(), None);
    assert_eq!(database.read().unwrap().get(b"key-0044").unwrap(), None);
    assert_eq!(
        database.read().unwrap().get(b"key-0300").unwrap(),
        Some(b"updated again".to_vec())
    );
    let stats = database.operational_stats().unwrap();
    assert_eq!(stats.snapshot.entries, 300);
    assert!(stats.writes.write_buffer_bytes > 0);
    assert!(stats.writes.data_page_bytes > 0);
    assert!(stats.writes.metadata_bytes > 0);
    assert_eq!(stats.checkpoint_failures, 0);
    let checkpoints = database.checkpoint_stats().unwrap();
    assert!(checkpoints.started > 0);
    assert_eq!(checkpoints.in_progress, 0);
    assert!(checkpoints.successful.operations > 0);
    assert_eq!(checkpoints.failures, 0);
    assert!(checkpoints.database_bytes > 0);
    drop(before_update);
    drop(empty);
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 300);
    assert_eq!(
        reopened.read().unwrap().get(b"key-0042").unwrap(),
        Some(b"updated".to_vec())
    );
    assert_eq!(reopened.read().unwrap().get(b"key-0043").unwrap(), None);
    assert_eq!(reopened.read().unwrap().get(b"key-0044").unwrap(), None);
    assert_eq!(
        reopened.read().unwrap().get(b"key-0301").unwrap(),
        Some(b"inserted".to_vec())
    );
}

#[test]
fn occupied_main_file_lease_defers_automatic_checkpoint() {
    let file = TestFile::new("buffer-checkpoint-lease");
    let database =
        Database::open_with_options(file.path(), DatabaseOptions::new().write_buffer_bytes(1))
            .unwrap();
    let initial_main_bytes = std::fs::metadata(file.path()).unwrap().len();
    let lease = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(file.path())
        .unwrap();
    lease.lock().unwrap();

    let mut first = database.write().unwrap();
    first.put(b"first", b"one").unwrap();
    first.commit().unwrap();
    let deferred = database.operational_stats().unwrap();
    assert_eq!(
        std::fs::metadata(file.path()).unwrap().len(),
        initial_main_bytes
    );
    assert_eq!(deferred.writes.data_page_bytes, 0);
    assert_eq!(deferred.checkpoint_failures, 0);

    lease.unlock().unwrap();
    let mut second = database.write().unwrap();
    second.put(b"second", b"two").unwrap();
    second.commit().unwrap();
    database.flush().unwrap();
    let completed = database.operational_stats().unwrap();
    assert!(std::fs::metadata(file.path()).unwrap().len() > initial_main_bytes);
    assert!(completed.writes.data_page_bytes > 0);
    assert_eq!(completed.checkpoint_failures, 0);
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 2);
    assert_eq!(
        reopened.read().unwrap().get(b"first").unwrap(),
        Some(b"one".to_vec())
    );
    assert_eq!(
        reopened.read().unwrap().get(b"second").unwrap(),
        Some(b"two".to_vec())
    );
}

#[test]
fn explicit_flush_consolidates_buffer_without_changing_snapshots() {
    let file = TestFile::new("buffer-explicit-flush");
    let database = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let mut first = database.write().unwrap();
    first.put(b"alpha", b"one").unwrap();
    first.put(b"beta", b"two").unwrap();
    first.commit().unwrap();
    let old_reader = database.read().unwrap();
    let mut second = database.write().unwrap();
    second.put(b"alpha", b"updated").unwrap();
    second.delete(b"beta").unwrap();
    second.put(b"gamma", b"three").unwrap();
    let transaction = second.commit().unwrap();
    let logical_hash = database.verify().unwrap().logical_hash;
    let wal = sidecar(file.path(), ".wal");
    let before_flush = std::fs::metadata(&wal).unwrap().len();

    database.flush().unwrap();
    let after_flush = std::fs::metadata(&wal).unwrap().len();
    database.flush().unwrap();
    let checkpoints = database.checkpoint_stats().unwrap();
    assert_eq!(checkpoints.started, 0);
    assert_eq!(checkpoints.in_progress, 0);
    assert_eq!(checkpoints.successful.operations, 0);
    assert_eq!(checkpoints.failures, 0);
    assert_eq!(checkpoints.database_bytes, 0);

    assert_eq!(database.stats().unwrap().transaction_id, transaction);
    assert_eq!(old_reader.get(b"alpha").unwrap(), Some(b"one".to_vec()));
    assert_eq!(old_reader.get(b"beta").unwrap(), Some(b"two".to_vec()));
    assert_eq!(
        database.read().unwrap().get(b"alpha").unwrap(),
        Some(b"updated".to_vec())
    );
    assert_eq!(database.read().unwrap().get(b"beta").unwrap(), None);
    let stats = database.operational_stats().unwrap();
    assert_eq!(stats.snapshot.entries, 2);
    assert_eq!(stats.snapshot.logical_hash, logical_hash);
    assert_eq!(stats.commits.operations, 2);
    assert_eq!(stats.syncs.operations, 3);
    assert_eq!(stats.wal_preflights.operations, 2);
    assert_eq!(stats.wal_encodes.operations, 2);
    assert_eq!(stats.wal_writes.operations, 2);
    assert_eq!(stats.wal_syncs.operations, 2);
    assert_eq!(stats.wal_publications.operations, 2);
    assert!(stats.wal_preflights.total_nanos > 0);
    assert!(stats.wal_encodes.total_nanos > 0);
    assert!(stats.wal_writes.total_nanos > 0);
    assert!(stats.wal_syncs.total_nanos > 0);
    assert!(stats.wal_publications.total_nanos > 0);
    assert!(stats.writes.write_buffer_bytes > 0);
    assert_eq!(stats.writes.data_page_bytes, 0);
    assert!(after_flush < before_flush);

    let reopened = Database::open(file.path()).unwrap();
    reopened.flush().unwrap();
    assert_eq!(reopened.stats().unwrap().transaction_id, transaction);
    assert_eq!(reopened.verify().unwrap().logical_hash, logical_hash);
    assert_eq!(
        reopened.read().unwrap().get(b"gamma").unwrap(),
        Some(b"three".to_vec())
    );
}

#[cfg(unix)]
#[test]
fn a_handle_with_the_replaced_buffer_generation_must_refresh() {
    let file = TestFile::new("buffer-snapshot-generation");
    let options = DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024);
    let first = Database::open_with_options(file.path(), options.clone()).unwrap();
    let second = Database::open_with_options(file.path(), options).unwrap();
    let mut writer = first.write().unwrap();
    writer.put(b"key", b"one").unwrap();
    let transaction = writer.commit().unwrap();
    assert_eq!(second.refresh().unwrap(), transaction);

    first.flush().unwrap();
    let mut stale = second.write().unwrap();
    stale.put(b"stale", b"must-not-publish").unwrap();
    assert!(matches!(stale.commit(), Err(Error::WriteConflict)));

    assert_eq!(second.refresh().unwrap(), transaction);
    let mut current = second.write().unwrap();
    current.put(b"next", b"two").unwrap();
    current.commit().unwrap();
    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(
        reopened.read().unwrap().get(b"next").unwrap(),
        Some(b"two".to_vec())
    );
    assert_eq!(reopened.read().unwrap().get(b"stale").unwrap(), None);
}

#[test]
fn buffered_handles_reject_stale_writers_and_refresh() {
    let file = TestFile::new("buffered-conflict");
    let options = DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024);
    let first = Database::open_with_options(file.path(), options.clone()).unwrap();
    let second = Database::open_with_options(file.path(), options).unwrap();
    let mut current = first.write().unwrap();
    let mut stale = second.write().unwrap();
    current.put(b"first", b"value").unwrap();
    stale.put(b"stale", b"value").unwrap();

    let transaction = current.commit().unwrap();
    assert!(matches!(stale.commit(), Err(Error::WriteConflict)));
    assert_eq!(second.refresh().unwrap(), transaction);
    let mut next = second.write().unwrap();
    next.put(b"second", b"value").unwrap();
    next.commit().unwrap();

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 2);
    assert_eq!(reopened.read().unwrap().get(b"stale").unwrap(), None);
}

#[test]
fn direct_handle_cannot_publish_over_an_external_buffer() {
    let file = TestFile::new("direct-versus-buffered-conflict");
    let backup = TestFile::new("direct-versus-buffered-backup");
    let compacted = TestFile::new("direct-versus-buffered-compact");
    let direct = Database::open(file.path()).unwrap();
    let buffered = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();

    let mut buffered_writer = buffered.write().unwrap();
    buffered_writer.put(b"durable", b"buffered").unwrap();
    let transaction = buffered_writer.commit().unwrap();

    let mut stale_writer = direct.write().unwrap();
    stale_writer.put(b"stale", b"must-not-publish").unwrap();
    assert!(matches!(stale_writer.commit(), Err(Error::WriteConflict)));
    assert!(matches!(
        direct.backup(backup.path()),
        Err(Error::WriteConflict)
    ));
    assert!(matches!(
        direct.compact_to(compacted.path()),
        Err(Error::WriteConflict)
    ));
    assert!(matches!(direct.compact(), Err(Error::WriteConflict)));
    assert!(matches!(
        direct.operational_stats(),
        Err(Error::WriteConflict)
    ));

    assert_eq!(direct.refresh().unwrap(), transaction);
    assert_eq!(
        direct.read().unwrap().get(b"durable").unwrap(),
        Some(b"buffered".to_vec())
    );
    assert_eq!(direct.read().unwrap().get(b"stale").unwrap(), None);
}

#[test]
fn external_buffered_process_blocks_a_stale_direct_handle() {
    let file = TestFile::new("external-buffered-conflict");
    let direct = Database::open(file.path()).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("buffered_commit_worker")
        .env("WSLDB_BUFFER_WORKER_PATH", file.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());

    let mut stale = direct.write().unwrap();
    stale.put(b"stale", b"must-not-publish").unwrap();
    assert!(matches!(stale.commit(), Err(Error::WriteConflict)));
    direct.refresh().unwrap();
    assert_eq!(
        direct.read().unwrap().get(b"external").unwrap(),
        Some(b"durable".to_vec())
    );
    assert_eq!(direct.read().unwrap().get(b"stale").unwrap(), None);
}

#[test]
fn buffered_commit_worker() {
    let Some(path) = std::env::var_os("WSLDB_BUFFER_WORKER_PATH") else {
        return;
    };
    let database = Database::open_with_options(
        path,
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"external", b"durable").unwrap();
    writer.commit().unwrap();
}

#[test]
fn buffered_backup_and_compaction_materialize_the_logical_snapshot() {
    let source = TestFile::new("buffered-operations-source");
    let backup = TestFile::new("buffered-operations-backup");
    let compacted = TestFile::new("buffered-operations-compact");
    let database = Database::open_with_options(
        source.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
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
    let expected: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    let expected_hash = database.verify().unwrap().logical_hash;

    let backup_report = database.backup(backup.path()).unwrap();
    let compact_report = database.compact_to(compacted.path()).unwrap();
    assert_eq!(backup_report.snapshot.logical_hash, expected_hash);
    assert_eq!(
        compact_report.compacted_snapshot.logical_hash,
        expected_hash
    );
    for path in [backup.path(), compacted.path()] {
        let reopened = Database::open(path).unwrap();
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
}

#[test]
fn static_compaction_rotates_a_buffered_generation() {
    let file = TestFile::new("buffered-static-compact");
    let database = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"value").unwrap();
    writer.commit().unwrap();
    let old_reader = database.read().unwrap();

    let report = Database::compact_file(file.path()).unwrap();
    assert_eq!(report.compacted_snapshot.entries, 1);
    assert_eq!(old_reader.get(b"key").unwrap(), Some(b"value".to_vec()));
    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(
        reopened.read().unwrap().get(b"key").unwrap(),
        Some(b"value".to_vec())
    );
}

#[test]
fn buffered_mixed_mutations_match_an_ordered_model_across_checkpoints() {
    let file = TestFile::new("buffered-model");
    let database = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(32 * 1024),
    )
    .unwrap();
    let mut model = BTreeMap::new();
    let mut random = 0x4D59_5DF4_D0F3_3173_u64;
    for round in 0..80_u32 {
        let mut writer = database.write().unwrap();
        for _ in 0..80 {
            random = xorshift(random);
            let key = format!("key-{:04}", random % 1_000).into_bytes();
            if random % 5 == 0 {
                assert_eq!(writer.delete(&key).unwrap(), model.remove(&key).is_some());
            } else {
                let value = format!("value-{random:016x}").into_bytes();
                writer.put(&key, &value).unwrap();
                model.insert(key, value);
            }
        }
        writer.commit().unwrap();
        if round % 8 == 0 {
            let reader = database.read().unwrap();
            assert_eq!(reader.iter().unwrap().collect::<BTreeMap<_, _>>(), model);
            let expected = model
                .range(b"key-0200".to_vec()..b"key-0800".to_vec())
                .take(37)
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Vec<_>>();
            assert_eq!(
                reader
                    .range(b"key-0200", Some(b"key-0800"), 37)
                    .unwrap()
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(database.verify().unwrap().entries, model.len() as u64);
        }
    }
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(
        reopened
            .read()
            .unwrap()
            .iter()
            .unwrap()
            .collect::<BTreeMap<_, _>>(),
        model
    );
}

fn xorshift(mut value: u64) -> u64 {
    value ^= value << 13;
    value ^= value >> 7;
    value ^ (value << 17)
}

#[cfg(feature = "crash-injection")]
#[test]
fn interrupted_buffered_compaction_preserves_the_logical_snapshot() {
    let baseline = TestFile::new("buffered-compaction-crash-baseline");
    let database = Database::open_with_options(
        baseline.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..128_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.commit().unwrap();
    let expected: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    drop(database);
    for stage in [
        "compaction_ready",
        "compaction_wal_stage_sync",
        "compaction_wal_handoff_write",
        "compaction_wal_handoff_sync",
        "compaction_wal_handoff_rename",
        "compaction_wal_handoff_directory_sync",
        "compaction_rename",
        "buffered_compaction_publish",
        "wal_truncate",
    ] {
        let target = TestFile::new("buffered-compaction-crash-target");
        fs::copy(baseline.path(), target.path()).unwrap();
        fs::copy(
            sidecar(baseline.path(), ".wal"),
            sidecar(target.path(), ".wal"),
        )
        .unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("buffered_compaction_crash_worker")
            .arg("--nocapture")
            .env("WSLDB_BUFFER_COMPACTION_PATH", target.path())
            .env("WSLDB_FAILPOINT", stage)
            .env("WSLDB_FAIL_AFTER", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "failpoint {stage} was not reached");
        let recovered = Database::open(target.path()).unwrap();
        assert_eq!(
            recovered
                .read()
                .unwrap()
                .iter()
                .unwrap()
                .collect::<BTreeMap<_, _>>(),
            expected
        );
        recovered.verify().unwrap();
    }
}
#[cfg(feature = "crash-injection")]
#[test]
fn crash_after_wal_write_recovers_one_complete_snapshot() {
    let file = TestFile::new("buffered-wal-write-crash");
    let database = Database::open(file.path()).unwrap();
    let mut initial = database.write().unwrap();
    initial.put(b"key", b"old").unwrap();
    initial.commit().unwrap();
    drop(database);
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("buffered_wal_write_crash_worker")
        .arg("--nocapture")
        .env("WSLDB_BUFFER_WAL_WRITE_PATH", file.path())
        .env("WSLDB_FAILPOINT", "write_buffer_write")
        .env("WSLDB_FAIL_AFTER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "WAL write failpoint was not reached");

    let recovered = Database::open(file.path()).unwrap();
    recovered.verify().unwrap();
    let value = recovered.read().unwrap().get(b"key").unwrap();
    assert!(value == Some(b"old".to_vec()) || value == Some(b"new".to_vec()));
}

#[cfg(feature = "crash-injection")]
#[test]
fn buffered_wal_write_crash_worker() {
    let Some(path) = std::env::var_os("WSLDB_BUFFER_WAL_WRITE_PATH") else {
        return;
    };
    let database = Database::open_with_options(
        path,
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"new").unwrap();
    writer.commit().unwrap();
}

#[cfg(feature = "crash-injection")]
#[test]
fn buffered_compaction_crash_worker() {
    let Some(path) = std::env::var_os("WSLDB_BUFFER_COMPACTION_PATH") else {
        return;
    };
    Database::open(path).unwrap().compact().unwrap();
}

fn sidecar(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}
