#![cfg(feature = "fault-injection")]

mod support;

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};

use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error, WriteBatch};

const BUFFER_BYTES: u64 = 64 * 1024 * 1024;

#[test]
fn partial_direct_writes_preserve_the_previous_commit() {
    let baseline = committed_baseline("direct-write-failure");
    for fail_after in [1_u64, 4_095] {
        let target = copied_database(&baseline, "direct-write-target");
        run_worker(&target, "direct-partial", "positioned_write", fail_after);
        assert_value(&target, false, b"old");
    }
}

#[test]
fn partial_metadata_writes_report_an_ambiguous_outcome() {
    let baseline = committed_baseline("metadata-write-failure");
    for fail_after in [4_097_u64, 8_191] {
        let target = copied_database(&baseline, "metadata-write-target");
        run_worker(
            &target,
            "direct-metadata-partial",
            "positioned_write",
            fail_after,
        );
        assert_old_or_new(&target, false);
    }
}

#[test]
fn partial_metadata_writes_preserve_odd_export_heads() {
    let source = committed_baseline("odd-export-source");
    let backup = TestFile::new("odd-export-backup");
    let compacted = TestFile::new("odd-export-compacted");
    let buffered_source = TestFile::new("odd-buffered-export-source");
    let buffered_compacted = TestFile::new("odd-buffered-export-compacted");
    let database = Database::open(source.path()).unwrap();
    assert_eq!(database.stats().unwrap().transaction_id.value(), 1);
    database.backup(backup.path()).unwrap();
    database.compact_to(compacted.path()).unwrap();

    let buffered = Database::open_with_options(
        buffered_source.path(),
        DatabaseOptions::new().write_buffer_bytes(BUFFER_BYTES),
    )
    .unwrap();
    let mut writer = buffered.write().unwrap();
    writer.put(b"key", b"old").unwrap();
    writer.commit().unwrap();
    buffered.compact_to(buffered_compacted.path()).unwrap();

    for (exported, label) in [
        (&backup, "odd-backup-metadata-target"),
        (&compacted, "odd-compacted-metadata-target"),
        (&buffered_compacted, "odd-buffered-metadata-target"),
    ] {
        for fail_after in [4_097_u64, 8_191] {
            let target = copied_database(exported, label);
            run_worker(
                &target,
                "direct-metadata-partial",
                "positioned_write",
                fail_after,
            );
            assert_old_or_new(&target, false);
        }
    }
}

#[test]
fn partial_buffered_writes_preserve_the_previous_commit() {
    let baseline = committed_baseline("buffered-write-failure");
    for fail_after in [1_u64, 51, 67, 512] {
        let target = copied_database(&baseline, "buffered-write-target");
        run_worker(&target, "buffered-partial", "positioned_write", fail_after);
        assert_value(&target, true, b"old");
    }
}

#[test]
fn partial_group_writes_remove_every_complete_prefix() {
    let baseline = committed_baseline("group-write-failure");
    let target = copied_database(&baseline, "group-write-target");
    run_worker(&target, "group-partial", "positioned_write", 100);
    assert_value(&target, true, b"old");
}

#[test]
fn live_handles_can_commit_after_a_partial_write_error() {
    let baseline = committed_baseline("live-write-recovery");
    for (mode, buffered) in [("direct-recover", false), ("buffered-recover", true)] {
        let target = copied_database(&baseline, "live-write-recovery-target");
        run_worker(&target, mode, "positioned_write", 1);
        assert_value(&target, buffered, b"recovered");
    }
}

#[test]
fn commit_sync_failures_report_whether_the_outcome_is_ambiguous() {
    let baseline = committed_baseline("sync-failure");

    let data_sync = copied_database(&baseline, "data-sync-target");
    run_worker(&data_sync, "direct-definite", "data_sync", 0);
    assert_value(&data_sync, false, b"old");

    let metadata_sync = copied_database(&baseline, "metadata-sync-target");
    run_worker(&metadata_sync, "direct-ambiguous", "metadata_sync", 0);
    assert_value(&metadata_sync, false, b"new");

    let buffered_sync = copied_database(&baseline, "buffer-sync-target");
    run_worker(&buffered_sync, "buffered-ambiguous", "write_buffer_sync", 0);
    assert_value(&buffered_sync, true, b"new");

    let group_sync = copied_database(&baseline, "group-sync-target");
    run_worker(&group_sync, "group-ambiguous", "write_buffer_sync", 0);
    let recovered = open(group_sync.path(), true);
    assert_eq!(recovered.stats().unwrap().transaction_id.value(), 3);
    assert_eq!(
        recovered.read().unwrap().get(b"key").unwrap(),
        Some(b"new".to_vec())
    );
    assert_eq!(
        recovered.read().unwrap().get(b"group-second").unwrap(),
        Some(b"durable".to_vec())
    );
}

#[test]
fn empty_wal_retries_a_failed_directory_sync_before_commit() {
    let file = TestFile::new("wal-sidecar-directory-sync");
    drop(Database::open(file.path()).unwrap());

    for _ in 0..2 {
        run_worker(
            &file,
            "wal-sidecar-directory",
            "wal_sidecar_directory_sync",
            0,
        );
        assert_eq!(fs::metadata(wal_path(file.path())).unwrap().len(), 0);
    }

    let database = open(file.path(), true);
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"durable").unwrap();
    writer.commit().unwrap();
    drop(database);

    let recovered = Database::open(file.path()).unwrap();
    assert_eq!(
        recovered.read().unwrap().get(b"key").unwrap(),
        Some(b"durable".to_vec())
    );
    recovered.verify().unwrap();
}

#[test]
fn io_failure_worker() {
    let Some(mode) = std::env::var_os("WSLDB_IO_WORKER_MODE") else {
        return;
    };
    let path = std::env::var_os("WSLDB_IO_WORKER_PATH").unwrap();
    if mode == "wal-sidecar-directory" {
        let result = Database::open_with_options(
            Path::new(&path),
            DatabaseOptions::new().write_buffer_bytes(BUFFER_BYTES),
        );
        let error = match result {
            Err(Error::Io(error)) => error,
            Err(other) => panic!("expected WAL directory-sync failure, got {other}"),
            Ok(_) => panic!("WAL directory sync unexpectedly succeeded"),
        };
        assert_eq!(error.kind(), ErrorKind::StorageFull);
        return;
    }
    let buffered = matches!(
        mode.to_str(),
        Some(
            "buffered-partial"
                | "buffered-recover"
                | "buffered-ambiguous"
                | "group-partial"
                | "group-ambiguous"
        )
    );
    let database = open(Path::new(&path), buffered);
    let before = database.stats().unwrap().transaction_id;
    let result = if mode == "group-partial" || mode == "group-ambiguous" {
        let mut first = WriteBatch::new();
        first.put(b"key", b"new").unwrap();
        let mut second = WriteBatch::new();
        second.put(b"group-second", b"durable").unwrap();
        second.put(b"payload", &[42; 1_024]).unwrap();
        database.commit_group(vec![first, second])
    } else {
        let mut writer = database.write().unwrap();
        writer.put(b"key", b"new").unwrap();
        if mode == "buffered-partial" {
            writer.put(b"payload", &[42; 1_024]).unwrap();
        } else if mode == "buffered-recover" {
            writer.put(b"new-key", b"uncommitted").unwrap();
        }
        writer.commit()
    };

    match mode.to_str().unwrap() {
        "direct-partial" | "buffered-partial" | "group-partial" | "direct-definite" => {
            let Err(Error::Io(error)) = result else {
                panic!("expected a definite injected I/O failure, got {result:?}");
            };
            assert_eq!(error.kind(), ErrorKind::StorageFull);
            assert_eq!(database.stats().unwrap().transaction_id, before);
            if mode == "group-partial" {
                let mut retry = WriteBatch::new();
                retry.put(b"group-second", b"recovered").unwrap();
                assert!(database.commit_group(vec![retry]).unwrap() > before);
                assert_eq!(
                    database.read().unwrap().get(b"group-second").unwrap(),
                    Some(b"recovered".to_vec())
                );
            }
        }
        "direct-recover" | "buffered-recover" => {
            let Err(Error::Io(error)) = result else {
                panic!("expected a recoverable injected I/O failure, got {result:?}");
            };
            assert_eq!(error.kind(), ErrorKind::StorageFull);
            assert_eq!(database.stats().unwrap().transaction_id, before);
            if mode == "buffered-recover" {
                assert_eq!(database.read().unwrap().get(b"new-key").unwrap(), None);
            }
            let mut retry = database.write().unwrap();
            retry.put(b"key", b"recovered").unwrap();
            if mode == "buffered-recover" {
                retry.put(b"new-key", b"recovered").unwrap();
            }
            assert!(retry.commit().unwrap() > before);
            if mode == "buffered-recover" {
                assert_eq!(
                    database.read().unwrap().get(b"new-key").unwrap(),
                    Some(b"recovered".to_vec())
                );
            }
        }
        "direct-metadata-partial" => {
            let Err(Error::CommitOutcomeUnknown(error)) = result else {
                panic!("expected an ambiguous injected sync failure, got {result:?}");
            };
            assert_eq!(error.kind(), ErrorKind::StorageFull);
            database.refresh().unwrap();
            let value = database.read().unwrap().get(b"key").unwrap();
            assert!(value == Some(b"old".to_vec()) || value == Some(b"new".to_vec()));
        }
        "direct-ambiguous" | "buffered-ambiguous" => {
            let Err(Error::CommitOutcomeUnknown(error)) = result else {
                panic!("expected an ambiguous injected sync failure, got {result:?}");
            };
            assert_eq!(error.kind(), ErrorKind::StorageFull);
            assert_stale_write_conflicts(&database);
            assert!(database.refresh().unwrap() > before);
            assert_eq!(
                database.read().unwrap().get(b"key").unwrap(),
                Some(b"new".to_vec())
            );
        }
        "group-ambiguous" => {
            let Err(Error::CommitOutcomeUnknown(error)) = result else {
                panic!("expected an ambiguous injected sync failure, got {result:?}");
            };
            assert_eq!(error.kind(), ErrorKind::StorageFull);
            assert_stale_write_conflicts(&database);
            assert_eq!(database.refresh().unwrap().value(), before.value() + 2);
            assert_eq!(
                database.read().unwrap().get(b"group-second").unwrap(),
                Some(b"durable".to_vec())
            );
        }
        unexpected => panic!("unexpected worker mode: {unexpected}"),
    }
}

fn assert_stale_write_conflicts(database: &Database) {
    let mut stale = database.write().unwrap();
    stale.put(b"stale", b"must-not-publish").unwrap();
    let result = stale.commit();
    assert!(
        matches!(result, Err(Error::WriteConflict)),
        "stale write returned {result:?}"
    );
}

fn committed_baseline(label: &str) -> TestFile {
    let file = TestFile::new(label);
    let database = Database::open(file.path()).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"old").unwrap();
    writer.commit().unwrap();
    file
}

fn copied_database(source: &TestFile, label: &str) -> TestFile {
    let target = TestFile::new(label);
    fs::copy(source.path(), target.path()).unwrap();
    target
}

fn run_worker(target: &TestFile, mode: &str, stage: &str, fail_after_bytes: u64) {
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("io_failure_worker")
        .arg("--nocapture")
        .env("WSLDB_IO_WORKER_PATH", target.path())
        .env("WSLDB_IO_WORKER_MODE", mode)
        .env("WSLDB_IO_FAILPOINT", stage)
        .env("WSLDB_IO_FAIL_AFTER_BYTES", fail_after_bytes.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "I/O failure worker failed for {mode}/{stage}"
    );
}

fn assert_value(file: &TestFile, buffered: bool, expected: &[u8]) {
    let database = open(file.path(), buffered);
    database.verify().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"key").unwrap(),
        Some(expected.to_vec())
    );
}

fn assert_old_or_new(file: &TestFile, buffered: bool) {
    let database = open(file.path(), buffered);
    database.verify().unwrap();
    let value = database.read().unwrap().get(b"key").unwrap();
    assert!(value == Some(b"old".to_vec()) || value == Some(b"new".to_vec()));
}

fn open(path: &Path, buffered: bool) -> Database {
    let options =
        DatabaseOptions::new().write_buffer_bytes(if buffered { BUFFER_BYTES } else { 0 });
    Database::open_with_options(path, options).unwrap()
}

fn wal_path(path: &Path) -> std::path::PathBuf {
    let mut wal = path.as_os_str().to_owned();
    wal.push(".wal");
    wal.into()
}
