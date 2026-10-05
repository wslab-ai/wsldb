#![cfg(feature = "crash-injection")]

mod support;

use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

use support::TestFile;
use wsldb::Database;

const MAX_WRITE_BOUNDARIES: u64 = 128;

#[test]
fn interrupted_backup_never_publishes_a_partial_target() {
    let source = committed_source("backup-crash-source");
    let expected = logical_hash(source.path());

    assert_all_prepublication_writes(&source, "backup", "snapshot_copy_write", expected);
    assert_prepublication_crash(&source, "backup", "backup_sync");
    assert_eq!(logical_hash(source.path()), expected);
}

#[test]
fn interrupted_compaction_never_publishes_a_partial_target() {
    let source = committed_source("compaction-crash-source");
    let expected = logical_hash(source.path());

    assert_all_prepublication_writes(&source, "compaction", "compaction_page_write", expected);
    for stage in [
        "compaction_data_sync",
        "compaction_metadata_write",
        "compaction_metadata_sync",
        "compaction_truncate",
        "compaction_final_sync",
    ] {
        assert_prepublication_crash(&source, "compaction", stage);
    }
    assert_eq!(logical_hash(source.path()), expected);
}

#[test]
fn interrupted_snapshot_publication_exposes_only_a_complete_target() {
    let source = committed_source("publication-crash-source");
    let expected = logical_hash(source.path());

    for stage in [
        "snapshot_publish_link",
        "snapshot_publish_directory_sync",
        "snapshot_temporary_remove",
        "snapshot_cleanup_directory_sync",
    ] {
        let target = TestFile::new("publication-crash-target");
        let status = run_worker(&source, &target, "compaction", stage, 1);
        assert!(!status.success(), "failpoint {stage} was not reached");
        assert_eq!(logical_hash(target.path()), expected, "stage {stage}");
    }
}

#[test]
fn maintenance_crash_worker() {
    let Some(source) = std::env::var_os("WSLDB_MAINTENANCE_CRASH_SOURCE") else {
        return;
    };
    let target = std::env::var_os("WSLDB_MAINTENANCE_CRASH_TARGET").unwrap();
    let mode = std::env::var("WSLDB_MAINTENANCE_CRASH_MODE").unwrap();
    let database = Database::open(Path::new(&source)).unwrap();
    match mode.as_str() {
        "backup" => {
            database.backup(Path::new(&target)).unwrap();
        }
        "compaction" => {
            database.compact_to(Path::new(&target)).unwrap();
        }
        _ => panic!("unexpected maintenance crash mode: {mode}"),
    }
}

fn assert_all_prepublication_writes(
    source: &TestFile,
    mode: &str,
    stage: &str,
    expected: [u8; 32],
) {
    let mut reached_end = false;
    for failure_after in 1..=MAX_WRITE_BOUNDARIES {
        let target = TestFile::new("maintenance-write-crash-target");
        let status = run_worker(source, &target, mode, stage, failure_after);
        if status.success() {
            assert_eq!(logical_hash(target.path()), expected);
            reached_end = true;
            break;
        }
        assert!(
            !target.path().exists(),
            "partial target published at {stage} hit {failure_after}"
        );
    }
    assert!(
        reached_end,
        "more than {MAX_WRITE_BOUNDARIES} {stage} hooks"
    );
}

fn assert_prepublication_crash(source: &TestFile, mode: &str, stage: &str) {
    let target = TestFile::new("maintenance-stage-crash-target");
    let status = run_worker(source, &target, mode, stage, 1);
    assert!(!status.success(), "failpoint {stage} was not reached");
    assert!(
        !target.path().exists(),
        "partial target published at {stage}"
    );
}

fn run_worker(
    source: &TestFile,
    target: &TestFile,
    mode: &str,
    stage: &str,
    failure_after: u64,
) -> ExitStatus {
    Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("maintenance_crash_worker")
        .arg("--nocapture")
        .env("WSLDB_MAINTENANCE_CRASH_SOURCE", source.path())
        .env("WSLDB_MAINTENANCE_CRASH_TARGET", target.path())
        .env("WSLDB_MAINTENANCE_CRASH_MODE", mode)
        .env("WSLDB_FAILPOINT", stage)
        .env("WSLDB_FAIL_AFTER", failure_after.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
}

fn committed_source(label: &str) -> TestFile {
    let source = TestFile::new(label);
    let database = Database::open(source.path()).unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..512_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.put(b"overflow", &[42; 25_000]).unwrap();
    writer.commit().unwrap();
    source
}

fn logical_hash(path: &Path) -> [u8; 32] {
    Database::open(path).unwrap().verify().unwrap().logical_hash
}
