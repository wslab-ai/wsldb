#![cfg(all(feature = "crash-injection", unix))]

#[path = "support/process.rs"]
mod process;
mod support;

use std::fs::File;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use process::{wait_for_path, wait_for_success};
use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error};

const THRESHOLD: u64 = 4 * 1024;

#[test]
fn external_commit_prevents_stale_checkpoint_publication() {
    let file = TestFile::new("freeze-swap-race");
    let ready = TestFile::new("freeze-swap-race-ready");
    let proceed = TestFile::new("freeze-swap-race-proceed");
    let external_done = TestFile::new("freeze-swap-race-external");
    let mut child = spawn_worker(
        file.path(),
        ready.path(),
        proceed.path(),
        external_done.path(),
    );
    wait_for_path(&mut child, ready.path(), "freeze-and-swap race worker");

    let external = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(THRESHOLD),
    )
    .unwrap();
    let mut writer = external.write().unwrap();
    writer.put(b"external", b"must survive").unwrap();
    writer.commit().unwrap();
    File::create(external_done.path())
        .unwrap()
        .sync_all()
        .unwrap();
    wait_for_success(&mut child, "freeze-and-swap race worker");
    drop(external);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(
        reopened.read().unwrap().get(b"external").unwrap(),
        Some(b"must survive".to_vec())
    );
    assert_eq!(reopened.verify().unwrap().entries, 257);
}

#[test]
fn freeze_swap_race_worker() {
    let Some(path) = std::env::var_os("WSLDB_FREEZE_SWAP_RACE_PATH") else {
        return;
    };
    let proceed = std::env::var_os("WSLDB_PAUSE_CONTINUE").unwrap();
    let external_done = std::env::var_os("WSLDB_EXTERNAL_DONE").unwrap();
    let database =
        Database::open_with_options(path, DatabaseOptions::new().write_buffer_bytes(THRESHOLD))
            .unwrap();
    let mut initial = database.write().unwrap();
    for index in 0..256_u32 {
        initial
            .put(format!("key-{index:04}").as_bytes(), &[index as u8; 128])
            .unwrap();
    }
    initial.commit().unwrap();

    wait_for_marker(Path::new(&external_done));
    File::create(proceed).unwrap().sync_all().unwrap();
    assert!(matches!(database.flush(), Err(Error::WriteConflict)));
    database.refresh().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"external").unwrap(),
        Some(b"must survive".to_vec())
    );
    database.verify().unwrap();
}

fn spawn_worker(database: &Path, ready: &Path, proceed: &Path, external_done: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("freeze_swap_race_worker")
        .arg("--nocapture")
        .env("WSLDB_FREEZE_SWAP_RACE_PATH", database)
        .env("WSLDB_PAUSEPOINT", "checkpoint_data_sync")
        .env("WSLDB_PAUSE_READY", ready)
        .env("WSLDB_PAUSE_CONTINUE", proceed)
        .env("WSLDB_EXTERNAL_DONE", external_done)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_for_marker(path: &Path) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "external writer did not publish its marker"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
