mod support;

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error};

#[test]
fn refresh_observes_an_external_process_commit() {
    let file = TestFile::new("refresh-external-commit");
    let database = database_with_initial_value(file.path());
    let old_reader = database.read().unwrap();
    let initial_transaction = old_reader.transaction_id();
    let mut stale_writer = database.write().unwrap();
    stale_writer.put(b"stale", b"must-not-commit").unwrap();

    let output = run_worker(
        "refresh_commit_worker",
        "WSLDB_REFRESH_COMMIT_WORKER_PATH",
        file.path(),
    );
    assert_worker_succeeded(output);

    assert!(matches!(stale_writer.commit(), Err(Error::WriteConflict)));
    assert_eq!(
        database.stats().unwrap().transaction_id,
        initial_transaction
    );
    assert_eq!(database.read().unwrap().get(b"external").unwrap(), None);

    let latest_transaction = Database::open(file.path())
        .unwrap()
        .stats()
        .unwrap()
        .transaction_id;
    assert_eq!(database.refresh().unwrap(), latest_transaction);
    assert_eq!(old_reader.get(b"external").unwrap(), None);
    assert_eq!(
        database.read().unwrap().get(b"external").unwrap(),
        Some(b"committed-in-worker".to_vec())
    );

    let mut writer = database.write().unwrap();
    writer.put(b"parent", b"committed-after-refresh").unwrap();
    writer.commit().unwrap();
    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 3);
    assert_eq!(
        reopened.read().unwrap().get(b"parent").unwrap(),
        Some(b"committed-after-refresh".to_vec())
    );
}

#[test]
fn refresh_commit_worker() {
    let Some(path) = std::env::var_os("WSLDB_REFRESH_COMMIT_WORKER_PATH") else {
        return;
    };
    let database = Database::open(path).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"external", b"committed-in-worker").unwrap();
    writer.commit().unwrap();
}

#[test]
fn buffered_refresh_replays_a_complete_tail_and_discards_an_incomplete_suffix() {
    let file = TestFile::new("buffered-incremental-refresh");
    let options = DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024);
    let first = Database::open_with_options(file.path(), options.clone()).unwrap();
    let second = Database::open_with_options(file.path(), options).unwrap();
    let old_reader = second.read().unwrap();

    let mut writer = first.write().unwrap();
    writer.put(b"external", b"complete").unwrap();
    let committed = writer.commit().unwrap();
    let wal = wal_path(file.path());
    let valid_bytes = std::fs::metadata(&wal).unwrap().len();
    let mut raw = OpenOptions::new().append(true).open(&wal).unwrap();
    raw.write_all(b"torn").unwrap();
    raw.sync_all().unwrap();
    drop(raw);

    assert_eq!(second.refresh().unwrap(), committed);
    assert_eq!(std::fs::metadata(&wal).unwrap().len(), valid_bytes);
    assert_eq!(old_reader.get(b"external").unwrap(), None);
    assert_eq!(
        second.read().unwrap().get(b"external").unwrap(),
        Some(b"complete".to_vec())
    );
    drop(first);
    drop(second);

    let reopened = Database::open(file.path()).unwrap();
    reopened.verify().unwrap();
    assert_eq!(
        reopened.read().unwrap().get(b"external").unwrap(),
        Some(b"complete".to_vec())
    );
}

#[cfg(unix)]
#[test]
fn refresh_observes_an_external_process_compaction_generation() {
    let file = TestFile::new("refresh-external-compaction");
    let database = database_with_initial_value(file.path());
    let old_reader = database.read().unwrap();
    let initial_transaction = old_reader.transaction_id();
    let mut stale_writer = database.write().unwrap();
    stale_writer.put(b"stale", b"must-not-commit").unwrap();

    let output = run_worker(
        "refresh_compaction_worker",
        "WSLDB_REFRESH_COMPACTION_WORKER_PATH",
        file.path(),
    );
    assert_worker_succeeded(output);

    assert!(matches!(stale_writer.commit(), Err(Error::WriteConflict)));
    assert_eq!(
        database.stats().unwrap().transaction_id,
        initial_transaction
    );
    assert_eq!(
        database.read().unwrap().get(b"after-compact").unwrap(),
        None
    );

    let latest_transaction = Database::open(file.path())
        .unwrap()
        .stats()
        .unwrap()
        .transaction_id;
    assert_eq!(database.refresh().unwrap(), latest_transaction);
    assert_eq!(old_reader.get(b"before-compact").unwrap(), None);
    assert_eq!(old_reader.get(b"after-compact").unwrap(), None);
    let refreshed_reader = database.read().unwrap();
    assert_eq!(
        refreshed_reader.get(b"before-compact").unwrap(),
        Some(b"worker-before".to_vec())
    );
    assert_eq!(
        refreshed_reader.get(b"after-compact").unwrap(),
        Some(b"worker-after".to_vec())
    );

    let mut writer = database.write().unwrap();
    writer.put(b"parent", b"after-generation-refresh").unwrap();
    writer.commit().unwrap();
    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 4);
    assert_eq!(
        reopened.read().unwrap().get(b"parent").unwrap(),
        Some(b"after-generation-refresh".to_vec())
    );
}

#[cfg(unix)]
#[test]
fn refresh_compaction_worker() {
    let Some(path) = std::env::var_os("WSLDB_REFRESH_COMPACTION_WORKER_PATH") else {
        return;
    };
    let database = Database::open(path).unwrap();
    let mut before = database.write().unwrap();
    before.put(b"before-compact", b"worker-before").unwrap();
    before.commit().unwrap();
    database.compact().unwrap();
    let mut after = database.write().unwrap();
    after.put(b"after-compact", b"worker-after").unwrap();
    after.commit().unwrap();
}

fn database_with_initial_value(path: &Path) -> Database {
    let database = Database::open(path).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"initial", b"parent-before-worker").unwrap();
    writer.commit().unwrap();
    database
}

fn run_worker(test: &str, variable: &str, path: &Path) -> Output {
    Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(test)
        .arg("--nocapture")
        .env(variable, path)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn assert_worker_succeeded(output: Output) {
    assert!(
        output.status.success(),
        "worker failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn wal_path(database: &Path) -> PathBuf {
    let mut path = OsString::from(database.as_os_str());
    path.push(".wal");
    PathBuf::from(path)
}
