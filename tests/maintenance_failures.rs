#![cfg(feature = "fault-injection")]

mod support;

use std::fmt::Debug;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error, Result};

const BUFFER_BYTES: u64 = 64 * 1024 * 1024;
const METADATA_BYTES: u64 = 2 * 4_096;

#[test]
fn partial_backup_and_compaction_writes_do_not_publish_targets() {
    let source = committed_baseline("partial-maintenance-source");
    let expected = logical_hash(source.path(), false);

    for fail_after in [1, 4_097] {
        let target = TestFile::new("partial-backup-target");
        run_worker(
            &source,
            Some(&target),
            "backup-definite",
            "positioned_write",
            Some(fail_after),
        );
        assert!(!target.path().exists(), "partial backup was published");
    }

    let probe = TestFile::new("partial-compaction-probe");
    let report = Database::open(source.path())
        .unwrap()
        .compact_to(probe.path())
        .unwrap();
    let data_bytes = report.compacted_bytes - METADATA_BYTES;
    for fail_after in [1, data_bytes + 1] {
        let target = TestFile::new("partial-compaction-target");
        run_worker(
            &source,
            Some(&target),
            "compact-to-definite",
            "positioned_write",
            Some(fail_after),
        );
        assert!(!target.path().exists(), "partial compaction was published");
    }

    assert_eq!(logical_hash(source.path(), false), expected);
}

#[test]
fn compact_output_failures_do_not_publish_a_target() {
    let source = committed_baseline("compact-output-source");
    let expected = logical_hash(source.path(), false);
    for stage in [
        "compaction_data_sync",
        "compaction_metadata_write",
        "compaction_metadata_sync",
        "compaction_truncate",
        "compaction_final_sync",
        "snapshot_publish_link",
    ] {
        let target = TestFile::new("compact-output-target");
        run_worker(&source, Some(&target), "compact-to-definite", stage, None);
        assert!(!target.path().exists(), "target published at {stage}");
        assert_eq!(logical_hash(source.path(), false), expected);
    }
}

#[test]
fn linked_snapshot_failures_report_ambiguous_publication() {
    let source = committed_baseline("snapshot-publish-source");
    let expected = logical_hash(source.path(), false);
    for stage in [
        "snapshot_publish_directory_sync",
        "snapshot_temporary_remove",
        "snapshot_cleanup_directory_sync",
    ] {
        let target = TestFile::new("snapshot-publish-target");
        run_worker(&source, Some(&target), "compact-to-ambiguous", stage, None);
        assert_eq!(logical_hash(target.path(), false), expected);
    }
}

#[test]
fn backup_sync_failure_leaves_source_intact_and_target_absent() {
    let source = committed_baseline("backup-failure-source");
    let expected = logical_hash(source.path(), false);
    let target = TestFile::new("backup-failure-target");
    run_worker(
        &source,
        Some(&target),
        "backup-definite",
        "backup_sync",
        None,
    );
    assert!(!target.path().exists());
    assert_eq!(logical_hash(source.path(), false), expected);
}

#[cfg(unix)]
#[test]
fn active_replacement_distinguishes_pre_and_post_rename_failures() {
    let baseline = committed_baseline("active-replace-source");
    let expected = logical_hash(baseline.path(), false);

    let before = copied_database(&baseline, "active-replace-before");
    run_worker(&before, None, "active-definite", "active_replace", None);
    assert_eq!(logical_hash(before.path(), false), expected);

    let after = copied_database(&baseline, "active-replace-after");
    run_worker(
        &after,
        None,
        "active-ambiguous",
        "active_replace_directory_sync",
        None,
    );
    assert_eq!(logical_hash(after.path(), false), expected);
}

#[test]
fn automatic_checkpoint_failures_keep_the_wal_commit_durable() {
    let empty = TestFile::new("streaming-checkpoint-failure-source");
    drop(Database::open(empty.path()).unwrap());
    let streaming_target = copied_database(&empty, "streaming-checkpoint-failure-target");
    run_worker(
        &streaming_target,
        None,
        "checkpoint",
        "checkpoint_stream_write",
        None,
    );
    assert_value(streaming_target.path(), true, b"checkpointed");

    let baseline = committed_baseline("checkpoint-failure-source");
    for stage in [
        "checkpoint_data_sync",
        "checkpoint_metadata_write",
        "checkpoint_metadata_sync",
    ] {
        let target = copied_database(&baseline, "checkpoint-failure-target");
        run_worker(&target, None, "checkpoint", stage, None);
        assert_value(target.path(), true, b"checkpointed");
    }
}

#[test]
fn checkpoint_wal_clear_failures_report_ambiguous_publication() {
    let baseline = committed_baseline("checkpoint-clear-failure-source");
    for stage in ["checkpoint_wal_clear", "wal_truncate_sync"] {
        let target = copied_database(&baseline, "checkpoint-clear-failure-target");
        run_worker(&target, None, "checkpoint-clear", stage, None);
        assert_value(target.path(), true, b"checkpointed");
    }
}

#[cfg(unix)]
#[test]
fn buffered_compaction_wal_handoff_sync_failure_reports_ambiguous_publication() {
    let baseline = TestFile::new("buffered-compaction-handoff-source");
    let database = open(baseline.path(), true);
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"compacted").unwrap();
    writer.commit().unwrap();
    drop(database);

    run_worker(
        &baseline,
        None,
        "active-buffered-ambiguous",
        "compaction_wal_handoff_directory_sync",
        None,
    );
    assert_value(baseline.path(), true, b"compacted");
}

#[cfg(unix)]
#[test]
fn buffer_snapshot_failures_preserve_data_and_require_refresh_after_rename() {
    let baseline = committed_baseline("buffer-snapshot-failure-source");
    for stage in ["write_buffer_snapshot_sync", "write_buffer_snapshot_rename"] {
        let target = copied_database(&baseline, "buffer-snapshot-definite");
        run_worker(&target, None, "flush-definite", stage, None);
        assert_value(target.path(), true, b"buffered");
    }

    let target = copied_database(&baseline, "buffer-snapshot-ambiguous");
    run_worker(
        &target,
        None,
        "flush-ambiguous",
        "write_buffer_snapshot_directory_sync",
        None,
    );
    assert_value(target.path(), true, b"after-refresh");
}

#[cfg(unix)]
#[test]
fn partial_buffer_snapshot_writes_leave_the_original_wal_authoritative() {
    const VALUE_BYTES: usize = 600 * 1024;

    let baseline = TestFile::new("partial-buffer-snapshot-source");
    let database = open(baseline.path(), true);
    let value = vec![0xa5; VALUE_BYTES];
    let mut writer = database.write().unwrap();
    writer.put(b"key", &value).unwrap();
    writer.commit().unwrap();
    drop(database);

    for fail_after in [1, 256 * 1024 + 1, 512 * 1024 + 1] {
        let target = TestFile::new("partial-buffer-snapshot-target");
        fs::copy(baseline.path(), target.path()).unwrap();
        fs::copy(
            sidecar(baseline.path(), ".wal"),
            sidecar(target.path(), ".wal"),
        )
        .unwrap();
        run_worker(
            &target,
            None,
            "flush-existing-definite",
            "positioned_write",
            Some(fail_after),
        );
        assert_value(target.path(), true, &value);
    }
}

#[test]
fn maintenance_failure_worker() {
    let Some(mode) = std::env::var_os("WSLDB_MAINTENANCE_WORKER_MODE") else {
        return;
    };
    let source = std::env::var_os("WSLDB_MAINTENANCE_SOURCE").unwrap();
    let source = Path::new(&source);
    let target = std::env::var_os("WSLDB_MAINTENANCE_TARGET");

    match mode.to_str().unwrap() {
        "compact-to-definite" => expect_io(
            Database::open(source)
                .unwrap()
                .compact_to(Path::new(target.as_ref().unwrap())),
        ),
        "compact-to-ambiguous" => expect_publication_unknown(
            Database::open(source)
                .unwrap()
                .compact_to(Path::new(target.as_ref().unwrap())),
        ),
        "backup-definite" => expect_io(
            Database::open(source)
                .unwrap()
                .backup(Path::new(target.as_ref().unwrap())),
        ),
        "active-definite" => expect_io(Database::compact_file(source)),
        "active-ambiguous" => expect_publication_unknown(Database::compact_file(source)),
        "active-buffered-ambiguous" => active_buffered_ambiguous_worker(source),
        "checkpoint" => checkpoint_worker(source),
        "checkpoint-clear" => checkpoint_clear_worker(source),
        "flush-definite" => flush_worker(source, false),
        "flush-ambiguous" => flush_worker(source, true),
        "flush-existing-definite" => expect_io(open(source, true).flush()),
        unexpected => panic!("unexpected maintenance worker mode: {unexpected}"),
    }
}

fn checkpoint_clear_worker(path: &Path) {
    let database =
        Database::open_with_options(path, DatabaseOptions::new().write_buffer_bytes(1)).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"checkpointed").unwrap();
    writer.commit().unwrap();
    expect_publication_unknown(database.flush());
    assert_stale_write_conflicts(&database);
    database.refresh().unwrap();
    commit_after_refresh(&database);
}

fn active_buffered_ambiguous_worker(path: &Path) {
    let database = open(path, true);
    expect_publication_unknown(database.compact());
    assert_stale_write_conflicts(&database);
    database.refresh().unwrap();
    commit_after_refresh(&database);
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

fn commit_after_refresh(database: &Database) {
    let mut current = database.write().unwrap();
    current.put(b"after-refresh", b"published").unwrap();
    current.commit().unwrap();
}

fn checkpoint_worker(path: &Path) {
    let database =
        Database::open_with_options(path, DatabaseOptions::new().write_buffer_bytes(1)).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"checkpointed").unwrap();
    writer.commit().unwrap();
    for _ in 0..100 {
        if database.checkpoint_stats().unwrap().failures == 1 {
            break;
        }
        database.write().unwrap().commit().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(database.checkpoint_stats().unwrap().failures, 1);
    assert_eq!(
        database.read().unwrap().get(b"key").unwrap(),
        Some(b"checkpointed".to_vec())
    );
}

fn flush_worker(path: &Path, ambiguous: bool) {
    let database = open(path, true);
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"buffered").unwrap();
    writer.commit().unwrap();
    if ambiguous {
        expect_publication_unknown(database.flush());
        database.refresh().unwrap();
        let mut after = database.write().unwrap();
        after.put(b"key", b"after-refresh").unwrap();
        after.commit().unwrap();
    } else {
        expect_io(database.flush());
    }
}

fn expect_io<T: Debug>(result: Result<T>) {
    assert!(matches!(result, Err(Error::Io(_))), "got {result:?}");
}

fn expect_publication_unknown<T: Debug>(result: Result<T>) {
    assert!(
        matches!(result, Err(Error::PublicationOutcomeUnknown(_))),
        "got {result:?}"
    );
}

fn committed_baseline(label: &str) -> TestFile {
    let file = TestFile::new(label);
    let database = Database::open(file.path()).unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..64_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.put(b"key", b"old").unwrap();
    writer.commit().unwrap();
    file
}

fn copied_database(source: &TestFile, label: &str) -> TestFile {
    let target = TestFile::new(label);
    fs::copy(source.path(), target.path()).unwrap();
    target
}

fn sidecar(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}

fn run_worker(
    source: &TestFile,
    target: Option<&TestFile>,
    mode: &str,
    stage: &str,
    fail_after_bytes: Option<u64>,
) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("--exact")
        .arg("maintenance_failure_worker")
        .arg("--nocapture")
        .env("WSLDB_MAINTENANCE_SOURCE", source.path())
        .env("WSLDB_MAINTENANCE_WORKER_MODE", mode)
        .env("WSLDB_IO_FAILPOINT", stage)
        .stdin(Stdio::null());
    if let Some(target) = target {
        command.env("WSLDB_MAINTENANCE_TARGET", target.path());
    }
    if let Some(bytes) = fail_after_bytes {
        command.env("WSLDB_IO_FAIL_AFTER_BYTES", bytes.to_string());
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "maintenance worker failed for {mode}/{stage}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn logical_hash(path: &Path, buffered: bool) -> [u8; 32] {
    open(path, buffered).verify().unwrap().logical_hash
}

fn assert_value(path: &Path, buffered: bool, expected: &[u8]) {
    let database = open(path, buffered);
    database.verify().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"key").unwrap(),
        Some(expected.to_vec())
    );
}

fn open(path: &Path, buffered: bool) -> Database {
    let options =
        DatabaseOptions::new().write_buffer_bytes(if buffered { BUFFER_BYTES } else { 0 });
    Database::open_with_options(path, options).unwrap()
}
