#![cfg(feature = "crash-injection")]

#[path = "support/process.rs"]
mod process;
mod support;

use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use process::{wait_for_path, wait_for_success};
use support::TestFile;
use wsldb::{Database, DatabaseOptions};

const BUFFER_BYTES: u64 = 64 * 1024 * 1024;

#[test]
fn concurrent_commit_does_not_change_a_direct_backup_snapshot() {
    run_race(WriteMode::Direct);
}

#[test]
fn concurrent_commit_does_not_change_a_buffered_backup_snapshot() {
    run_race(WriteMode::Buffered);
}

#[test]
fn backup_race_worker() {
    let Some(source) = std::env::var_os("WSLDB_BACKUP_RACE_SOURCE") else {
        return;
    };
    let target = std::env::var_os("WSLDB_BACKUP_RACE_TARGET").unwrap();
    Database::open(source).unwrap().backup(target).unwrap();
}

fn run_race(mode: WriteMode) {
    let source = TestFile::new(mode.label());
    let target = TestFile::new("backup-race-target");
    let database = mode.open(source.path());
    let mut initial = database.write().unwrap();
    for index in 0..512_u32 {
        initial
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    initial.put(b"overflow", &[29; 25_000]).unwrap();
    initial.commit().unwrap();
    let before = database.verify().unwrap();

    let ready = TestFile::new("backup-race-ready");
    let proceed = TestFile::new("backup-race-continue");
    let mut child = spawn_worker(
        source.path(),
        target.path(),
        ready.path(),
        proceed.path(),
        mode.pausepoint(),
    );
    wait_for_path(&mut child, ready.path(), "backup race worker");

    let mut concurrent = database.write().unwrap();
    concurrent.put(b"concurrent", b"source-only").unwrap();
    let committed = concurrent.commit().unwrap();
    File::create(proceed.path()).unwrap().sync_all().unwrap();
    wait_for_success(&mut child, "backup race worker");

    let source_after = database.verify().unwrap();
    assert_eq!(source_after.transaction_id, committed);
    assert_eq!(source_after.entries, before.entries + 1);
    assert_eq!(
        database.read().unwrap().get(b"concurrent").unwrap(),
        Some(b"source-only".to_vec())
    );

    let backup = Database::open(target.path()).unwrap();
    let backed_up = backup.verify().unwrap();
    assert_eq!(backed_up.transaction_id, before.transaction_id);
    assert_eq!(backed_up.entries, before.entries);
    assert_eq!(backed_up.logical_hash, before.logical_hash);
    assert_eq!(backup.read().unwrap().get(b"concurrent").unwrap(), None);
    assert_no_temporary_files(target.path());
}

fn spawn_worker(
    source: &Path,
    target: &Path,
    ready: &Path,
    proceed: &Path,
    pausepoint: &str,
) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("backup_race_worker")
        .arg("--nocapture")
        .env("WSLDB_BACKUP_RACE_SOURCE", source)
        .env("WSLDB_BACKUP_RACE_TARGET", target)
        .env("WSLDB_PAUSEPOINT", pausepoint)
        .env("WSLDB_PAUSE_READY", ready)
        .env("WSLDB_PAUSE_CONTINUE", proceed)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn assert_no_temporary_files(target: &Path) {
    let file_name = target.file_name().unwrap().to_string_lossy();
    let prefix = format!("{file_name}.wsldb-tmp-");
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let leftovers: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path())
        .collect();
    assert!(leftovers.is_empty(), "stale temporary files: {leftovers:?}");
}

#[derive(Clone, Copy)]
enum WriteMode {
    Direct,
    Buffered,
}

impl WriteMode {
    fn open(self, path: &Path) -> Database {
        match self {
            Self::Direct => Database::open(path).unwrap(),
            Self::Buffered => Database::open_with_options(
                path,
                DatabaseOptions::new().write_buffer_bytes(BUFFER_BYTES),
            )
            .unwrap(),
        }
    }

    const fn pausepoint(self) -> &'static str {
        match self {
            Self::Direct => "snapshot_copy_write",
            Self::Buffered => "compaction_page_write",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct-backup-race",
            Self::Buffered => "buffered-backup-race",
        }
    }
}
