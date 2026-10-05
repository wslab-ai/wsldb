#![cfg(feature = "crash-injection")]

mod support;

use std::fs::File;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use support::TestFile;
use wsldb::{Database, DatabaseOptions};

const THRESHOLD: u64 = 4 * 1024;

#[test]
fn commits_during_checkpoint_preparation_preserve_every_snapshot() {
    let Some(path) = std::env::var_os("WSLDB_FREEZE_SWAP_PATH") else {
        let database = TestFile::new("freeze-swap-database");
        let ready = TestFile::new("freeze-swap-ready");
        let proceed = TestFile::new("freeze-swap-proceed");
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("commits_during_checkpoint_preparation_preserve_every_snapshot")
            .arg("--nocapture")
            .env("WSLDB_FREEZE_SWAP_PATH", database.path())
            .env("WSLDB_PAUSEPOINT", "checkpoint_data_sync")
            .env("WSLDB_PAUSE_READY", ready.path())
            .env("WSLDB_PAUSE_CONTINUE", proceed.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "freeze-and-swap worker failed");
        return;
    };

    let ready = std::env::var_os("WSLDB_PAUSE_READY").unwrap();
    let proceed = std::env::var_os("WSLDB_PAUSE_CONTINUE").unwrap();
    let database =
        Database::open_with_options(path, DatabaseOptions::new().write_buffer_bytes(THRESHOLD))
            .unwrap();
    let empty = database.read().unwrap();

    let mut initial = database.write().unwrap();
    for index in 0..256_u32 {
        initial
            .put(format!("key-{index:04}").as_bytes(), &[index as u8; 128])
            .unwrap();
    }
    initial.commit().unwrap();
    let frozen = database.read().unwrap();
    wait_for_path(ready.as_ref());
    let preparing = database.checkpoint_stats().unwrap();
    assert_eq!(preparing.started, 1);
    assert_eq!(preparing.in_progress, 1);
    assert_eq!(preparing.successful.operations, 0);

    let mut suffix = database.write().unwrap();
    suffix
        .put(b"key-0042", b"updated during checkpoint")
        .unwrap();
    suffix.delete(b"key-0043").unwrap();
    suffix
        .put(b"key-0300", b"inserted during checkpoint")
        .unwrap();
    suffix.commit().unwrap();
    let during = database.read().unwrap();

    assert_eq!(empty.get(b"key-0042").unwrap(), None);
    assert_eq!(frozen.get(b"key-0042").unwrap(), Some(vec![42; 128]));
    assert_eq!(frozen.get(b"key-0043").unwrap(), Some(vec![43; 128]));
    assert_eq!(
        during.get(b"key-0042").unwrap(),
        Some(b"updated during checkpoint".to_vec())
    );
    assert_eq!(during.get(b"key-0043").unwrap(), None);
    assert_eq!(
        during.get(b"key-0300").unwrap(),
        Some(b"inserted during checkpoint".to_vec())
    );

    let stop = Arc::new(AtomicBool::new(false));
    let reader_database = database.clone();
    let reader_stop = Arc::clone(&stop);
    let concurrent_reader = thread::spawn(move || {
        let snapshot = reader_database.read().unwrap();
        while !reader_stop.load(Ordering::Acquire) {
            assert_eq!(snapshot.get(b"key-0043").unwrap(), None);
            assert_eq!(snapshot.iter().unwrap().count(), 256);
        }
    });
    File::create(proceed).unwrap().sync_all().unwrap();
    database.flush().unwrap();
    let completed = database.checkpoint_stats().unwrap();
    assert_eq!(completed.started, 1);
    assert_eq!(completed.in_progress, 0);
    assert_eq!(completed.successful.operations, 1);
    stop.store(true, Ordering::Release);
    concurrent_reader.join().unwrap();
    let published = database.read().unwrap();
    assert_eq!(
        published.get(b"key-0042").unwrap(),
        Some(b"updated during checkpoint".to_vec())
    );
    assert_eq!(published.get(b"key-0043").unwrap(), None);
    assert_eq!(
        published.get(b"key-0300").unwrap(),
        Some(b"inserted during checkpoint".to_vec())
    );
    assert_eq!(database.verify().unwrap().entries, 256);

    drop(published);
    drop(during);
    drop(frozen);
    drop(empty);
    drop(database);

    let reopened = Database::open(std::env::var_os("WSLDB_FREEZE_SWAP_PATH").unwrap()).unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 256);
    assert_eq!(
        reopened.read().unwrap().get(b"key-0042").unwrap(),
        Some(b"updated during checkpoint".to_vec())
    );
    assert_eq!(reopened.read().unwrap().get(b"key-0043").unwrap(), None);
    assert_eq!(
        reopened.read().unwrap().get(b"key-0300").unwrap(),
        Some(b"inserted during checkpoint".to_vec())
    );
}

#[test]
fn checkpoint_worker_panic_keeps_the_wal_authoritative() {
    let Some(path) = std::env::var_os("WSLDB_CHECKPOINT_PANIC_PATH") else {
        let database = TestFile::new("checkpoint-panic");
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("checkpoint_worker_panic_keeps_the_wal_authoritative")
            .arg("--nocapture")
            .env("WSLDB_CHECKPOINT_PANIC_PATH", database.path())
            .env("WSLDB_PANICPOINT", "checkpoint_data_sync")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "checkpoint panic worker failed");
        return;
    };

    let database =
        Database::open_with_options(path, DatabaseOptions::new().write_buffer_bytes(1)).unwrap();
    let mut first = database.write().unwrap();
    first.put(b"key", b"durable").unwrap();
    first.commit().unwrap();
    for _ in 0..100 {
        if database.checkpoint_stats().unwrap().failures == 1 {
            break;
        }
        database.write().unwrap().commit().unwrap();
        thread::sleep(Duration::from_millis(1));
    }
    let checkpoints = database.checkpoint_stats().unwrap();
    assert_eq!(checkpoints.started, 1);
    assert_eq!(checkpoints.in_progress, 0);
    assert_eq!(checkpoints.successful.operations, 0);
    assert_eq!(checkpoints.failures, 1);
    assert_eq!(
        database.read().unwrap().get(b"key").unwrap(),
        Some(b"durable".to_vec())
    );
    database.verify().unwrap();
}

#[test]
fn active_compaction_waits_for_and_retires_a_pending_worker() {
    let file = TestFile::new("checkpoint-compaction");
    let database =
        Database::open_with_options(file.path(), DatabaseOptions::new().write_buffer_bytes(1))
            .unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..256_u32 {
        writer
            .put(format!("key-{index:04}").as_bytes(), b"value")
            .unwrap();
    }
    writer.commit().unwrap();

    let report = database.compact().unwrap();
    assert_eq!(report.compacted_snapshot.entries, 256);
    assert_eq!(database.verify().unwrap().entries, 256);
    drop(database);
    assert_eq!(
        Database::open(file.path())
            .unwrap()
            .verify()
            .unwrap()
            .entries,
        256
    );
}

#[test]
fn dropping_the_last_handle_waits_for_a_pending_checkpoint_worker() {
    let Some(path) = std::env::var_os("WSLDB_CHECKPOINT_DROP_PATH") else {
        let database = TestFile::new("checkpoint-drop-database");
        let ready = TestFile::new("checkpoint-drop-ready");
        let proceed = TestFile::new("checkpoint-drop-proceed");
        let dropping = TestFile::new("checkpoint-drop-started");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("dropping_the_last_handle_waits_for_a_pending_checkpoint_worker")
            .arg("--nocapture")
            .env("WSLDB_CHECKPOINT_DROP_PATH", database.path())
            .env("WSLDB_CHECKPOINT_DROP_STARTED", dropping.path())
            .env("WSLDB_PAUSEPOINT", "checkpoint_data_sync")
            .env("WSLDB_PAUSE_READY", ready.path())
            .env("WSLDB_PAUSE_CONTINUE", proceed.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        wait_for_path(ready.path());
        wait_for_path(dropping.path());
        thread::sleep(Duration::from_millis(50));
        let waited = child.try_wait().unwrap().is_none();
        File::create(proceed.path()).unwrap().sync_all().unwrap();
        let status = child.wait().unwrap();

        assert!(waited, "database drop detached the checkpoint worker");
        assert!(status.success(), "checkpoint drop worker failed");
        return;
    };

    let database =
        Database::open_with_options(&path, DatabaseOptions::new().write_buffer_bytes(1)).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"durable").unwrap();
    writer.commit().unwrap();
    File::create(std::env::var_os("WSLDB_CHECKPOINT_DROP_STARTED").unwrap())
        .unwrap()
        .sync_all()
        .unwrap();
    drop(database);

    let reopened = Database::open(path).unwrap();
    assert_eq!(
        reopened.read().unwrap().get(b"key").unwrap(),
        Some(b"durable".to_vec())
    );
    reopened.verify().unwrap();
}

fn wait_for_path(path: &std::path::Path) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "checkpoint worker did not reach its pausepoint"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
