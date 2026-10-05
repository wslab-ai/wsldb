#![cfg(all(feature = "crash-injection", unix))]

#[path = "support/process.rs"]
mod process;
mod support;

use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Instant;

use process::{PROCESS_TIMEOUT, wait_for_path, wait_for_success};
use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error};

const BUFFER_BYTES: u64 = 64 * 1024 * 1024;

#[test]
fn concurrent_commit_prevents_stale_direct_compaction_publication() {
    run_race(WriteMode::Direct);
}

#[test]
fn concurrent_commit_prevents_stale_buffered_compaction_publication() {
    run_race(WriteMode::Buffered);
}

#[test]
fn byte_identical_main_replacement_prevents_compaction_publication() {
    for mode in [WriteMode::Direct, WriteMode::Buffered] {
        run_replacement_race(mode);
    }
}

#[test]
fn interrupted_wal_handoff_preserves_the_concurrent_commit() {
    for stage in [
        "compaction_wal_stage_sync",
        "compaction_wal_handoff_write",
        "compaction_wal_handoff_sync",
        "compaction_wal_handoff_rename",
        "compaction_wal_handoff_directory_sync",
        "compaction_rename",
        "buffered_compaction_publish",
    ] {
        let file = TestFile::new("wal-handoff-crash");
        let database = WriteMode::Buffered.open(file.path());
        let mut initial = database.write().unwrap();
        initial.put(b"initial", b"value").unwrap();
        initial.commit().unwrap();

        let ready = TestFile::new("wal-handoff-ready");
        let proceed = TestFile::new("wal-handoff-continue");
        let mut child = spawn_worker(
            file.path(),
            ready.path(),
            proceed.path(),
            WriteMode::Buffered,
            Some(stage),
            false,
            false,
        );
        wait_for_path(&mut child, ready.path(), "WAL handoff crash worker");
        let mut concurrent = database.write().unwrap();
        concurrent.put(b"concurrent", stage.as_bytes()).unwrap();
        concurrent.commit().unwrap();
        File::create(proceed.path()).unwrap().sync_all().unwrap();
        wait_for_failure(&mut child, "WAL handoff crash worker");
        drop(database);

        let recovered = Database::open(file.path()).unwrap();
        assert_eq!(
            recovered.read().unwrap().get(b"concurrent").unwrap(),
            Some(stage.as_bytes().to_vec()),
            "lost concurrent commit after {stage}"
        );
        assert_eq!(recovered.verify().unwrap().entries, 2);
    }
}

#[test]
fn wal_handoff_io_failures_preserve_every_committed_transaction() {
    for stage in [
        "compaction_wal_stage_sync",
        "compaction_wal_handoff_write",
        "compaction_wal_handoff_sync",
        "compaction_wal_handoff_rename",
        "compaction_wal_handoff_directory_sync",
    ] {
        let file = TestFile::new("wal-handoff-io");
        let database = WriteMode::Buffered.open(file.path());
        let mut initial = database.write().unwrap();
        initial.put(b"initial", b"value").unwrap();
        initial.commit().unwrap();

        let ready = TestFile::new("wal-handoff-io-ready");
        let proceed = TestFile::new("wal-handoff-io-continue");
        let mut child = spawn_worker(
            file.path(),
            ready.path(),
            proceed.path(),
            WriteMode::Buffered,
            Some(stage),
            true,
            false,
        );
        wait_for_path(&mut child, ready.path(), "WAL handoff I/O worker");
        let mut concurrent = database.write().unwrap();
        concurrent.put(b"concurrent", stage.as_bytes()).unwrap();
        concurrent.commit().unwrap();
        File::create(proceed.path()).unwrap().sync_all().unwrap();
        wait_for_success(&mut child, "WAL handoff I/O worker");
        drop(database);

        let recovered = Database::open(file.path()).unwrap();
        assert_eq!(
            recovered.read().unwrap().get(b"concurrent").unwrap(),
            Some(stage.as_bytes().to_vec())
        );
        assert_eq!(recovered.verify().unwrap().entries, 2);
    }
}

#[test]
fn compaction_race_worker() {
    let Some(path) = std::env::var_os("WSLDB_COMPACTION_RACE_PATH") else {
        return;
    };
    let result = Database::compact_file(path);
    if std::env::var_os("WSLDB_EXPECT_REPLACEMENT_CONFLICT").is_some() {
        assert!(
            matches!(result, Err(Error::WriteConflict)),
            "replacement race returned {result:?}"
        );
        return;
    }
    match std::env::var("WSLDB_COMPACTION_RACE_MODE").as_deref() {
        Ok("direct-compaction-race") => assert!(
            matches!(result, Err(Error::WriteConflict)),
            "stale direct compaction did not return a write conflict"
        ),
        Ok("buffered-compaction-race") => {
            if std::env::var_os("WSLDB_EXPECT_COMPACTION_IO").is_some() {
                assert!(
                    matches!(
                        result,
                        Err(Error::Io(_) | Error::PublicationOutcomeUnknown(_))
                    ),
                    "got {result:?}"
                );
            } else {
                result.unwrap();
            }
        }
        _ => panic!("compaction race mode is missing"),
    }
}

fn run_race(mode: WriteMode) {
    let file = TestFile::new(mode.label());
    let database = mode.open(file.path());
    let mut initial = database.write().unwrap();
    for index in 0..512_u32 {
        initial
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    initial.put(b"overflow", &[17; 25_000]).unwrap();
    initial.commit().unwrap();
    let before = database.verify().unwrap();

    let ready = TestFile::new("compaction-race-ready");
    let proceed = TestFile::new("compaction-race-continue");
    let mut child = spawn_worker(
        file.path(),
        ready.path(),
        proceed.path(),
        mode,
        None,
        false,
        false,
    );
    wait_for_path(&mut child, ready.path(), "compaction race worker");

    let mut concurrent = database.write().unwrap();
    concurrent.put(b"concurrent", b"must-survive").unwrap();
    let committed = concurrent.commit().unwrap();
    let expected_wal =
        (mode == WriteMode::Buffered).then(|| fs::read(sidecar(file.path(), ".wal")).unwrap());
    File::create(proceed.path()).unwrap().sync_all().unwrap();
    wait_for_success(&mut child, "compaction race worker");
    if let Some(expected_wal) = expected_wal {
        assert_eq!(
            fs::read(sidecar(file.path(), ".wal")).unwrap(),
            expected_wal
        );
    }

    assert_eq!(database.stats().unwrap().transaction_id, committed);
    assert_eq!(
        database.read().unwrap().get(b"concurrent").unwrap(),
        Some(b"must-survive".to_vec())
    );
    if mode == WriteMode::Buffered {
        let mut stale = database.write().unwrap();
        stale.put(b"stale", b"must-not-commit").unwrap();
        assert!(matches!(stale.commit(), Err(Error::WriteConflict)));
    }
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    let after = reopened.verify().unwrap();
    let expected_transaction = committed.value();
    assert_eq!(after.transaction_id.value(), expected_transaction);
    assert_eq!(after.entries, before.entries + 1);
    assert_eq!(
        reopened.read().unwrap().get(b"concurrent").unwrap(),
        Some(b"must-survive".to_vec())
    );
    assert_eq!(reopened.read().unwrap().get(b"stale").unwrap(), None);
    assert_no_replacement_files(file.path());
    assert_no_handoff_files(file.path());
}

fn run_replacement_race(mode: WriteMode) {
    let file = TestFile::new("compaction-replacement-race");
    let replacement = TestFile::new("compaction-replacement-copy");
    let database = mode.open(file.path());
    let mut initial = database.write().unwrap();
    initial.put(b"stable", b"must-survive").unwrap();
    initial.commit().unwrap();
    let before = database.verify().unwrap();

    let ready = TestFile::new("compaction-replacement-ready");
    let proceed = TestFile::new("compaction-replacement-continue");
    let mut child = spawn_worker(
        file.path(),
        ready.path(),
        proceed.path(),
        mode,
        None,
        false,
        true,
    );
    wait_for_path(&mut child, ready.path(), "replacement compaction worker");
    fs::copy(file.path(), replacement.path()).unwrap();
    fs::rename(replacement.path(), file.path()).unwrap();
    File::create(proceed.path()).unwrap().sync_all().unwrap();
    wait_for_success(&mut child, "replacement compaction worker");
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap(), before);
    assert_eq!(
        reopened.read().unwrap().get(b"stable").unwrap(),
        Some(b"must-survive".to_vec())
    );
    assert_no_replacement_files(file.path());
    assert_no_handoff_files(file.path());
}

fn spawn_worker(
    database: &Path,
    ready: &Path,
    proceed: &Path,
    mode: WriteMode,
    failpoint: Option<&str>,
    expect_io: bool,
    expect_replacement_conflict: bool,
) -> Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("--exact")
        .arg("compaction_race_worker")
        .arg("--nocapture")
        .env("WSLDB_COMPACTION_RACE_PATH", database)
        .env("WSLDB_COMPACTION_RACE_MODE", mode.label())
        .env("WSLDB_PAUSEPOINT", "compaction_ready")
        .env("WSLDB_PAUSE_READY", ready)
        .env("WSLDB_PAUSE_CONTINUE", proceed)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(failpoint) = failpoint {
        if expect_io {
            command
                .env("WSLDB_IO_FAILPOINT", failpoint)
                .env("WSLDB_IO_FAIL_AFTER", "1");
        } else {
            command
                .env("WSLDB_FAILPOINT", failpoint)
                .env("WSLDB_FAIL_AFTER", "1");
        }
    }
    if expect_io {
        command.env("WSLDB_EXPECT_COMPACTION_IO", "1");
    }
    if expect_replacement_conflict {
        command.env("WSLDB_EXPECT_REPLACEMENT_CONFLICT", "1");
    }
    command.spawn().unwrap()
}

fn wait_for_failure(child: &mut Child, context: &str) {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success(), "{context} unexpectedly succeeded");
            return;
        }
        if started.elapsed() >= PROCESS_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{context} did not finish");
        }
        thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn assert_no_replacement_files(database: &Path) {
    let file_name = database.file_name().unwrap().to_string_lossy();
    let prefix = format!("{file_name}.wsldb-replacement-");
    let parent = database.parent().unwrap_or_else(|| Path::new("."));
    let leftovers: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path())
        .collect();
    assert!(leftovers.is_empty(), "stale replacements: {leftovers:?}");
}

fn assert_no_handoff_files(database: &Path) {
    let file_name = database.file_name().unwrap().to_string_lossy();
    let prefix = format!("{file_name}.wal.wsldb-handoff-");
    let parent = database.parent().unwrap_or_else(|| Path::new("."));
    let leftovers: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path())
        .collect();
    assert!(leftovers.is_empty(), "stale WAL handoffs: {leftovers:?}");
}

fn sidecar(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}

#[derive(Clone, Copy, Eq, PartialEq)]
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

    const fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct-compaction-race",
            Self::Buffered => "buffered-compaction-race",
        }
    }
}
