#![cfg(feature = "crash-injection")]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use support::TestFile;
use wsldb::{Database, DatabaseOptions};

#[test]
fn interrupted_buffer_checkpoints_recover_the_durable_commit() {
    let empty = TestFile::new("streaming-buffer-crash-baseline");
    drop(Database::open(empty.path()).unwrap());
    let streaming_expected = (0..64_u32)
        .map(|index| {
            (
                format!("key-{index:04}").into_bytes(),
                format!("new-{index:04}").into_bytes(),
            )
        })
        .collect();
    assert!(
        run_interrupted_buffer_checkpoint(
            empty.path(),
            &streaming_expected,
            "checkpoint_stream_write",
            1,
        ),
        "streaming checkpoint write crash stage was not reached"
    );

    let baseline = TestFile::new("buffer-crash-baseline");
    let database = Database::open(baseline.path()).unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..64_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("old-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.commit().unwrap();
    let mut expected: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    for index in 0..64_u32 {
        expected.insert(
            format!("key-{index:04}").into_bytes(),
            format!("new-{index:04}").into_bytes(),
        );
    }
    drop(database);

    let mut reached_end = false;
    for failure_after in 1..=128_u64 {
        if !run_interrupted_buffer_checkpoint(
            baseline.path(),
            &expected,
            "page_write",
            failure_after,
        ) {
            reached_end = true;
            break;
        }
    }
    assert!(reached_end);
    for stage in [
        "write_buffer_sync",
        "checkpoint_data_sync",
        "checkpoint_metadata_write",
        "checkpoint_metadata_sync",
        "wal_truncate",
        "checkpoint_wal_clear",
    ] {
        assert!(
            run_interrupted_buffer_checkpoint(baseline.path(), &expected, stage, 1),
            "checkpoint crash stage was not reached: {stage}"
        );
    }
}

#[test]
fn buffered_checkpoint_crash_worker() {
    let Some(path) = std::env::var_os("WSLDB_BUFFER_CRASH_PATH") else {
        return;
    };
    let database =
        Database::open_with_options(path, DatabaseOptions::new().write_buffer_bytes(1)).unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..64_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("new-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.commit().unwrap();
    database.flush().unwrap();
}

fn run_interrupted_buffer_checkpoint(
    baseline: &Path,
    expected: &BTreeMap<Vec<u8>, Vec<u8>>,
    stage: &str,
    failure_after: u64,
) -> bool {
    let target = TestFile::new("buffer-crash-target");
    fs::copy(baseline, target.path()).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("buffered_checkpoint_crash_worker")
        .arg("--nocapture")
        .env("WSLDB_BUFFER_CRASH_PATH", target.path())
        .env("WSLDB_FAILPOINT", stage)
        .env("WSLDB_FAIL_AFTER", failure_after.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();

    let recovered = Database::open(target.path()).unwrap();
    let actual: BTreeMap<_, _> = recovered.read().unwrap().iter().unwrap().collect();
    assert_eq!(actual, *expected);
    recovered.verify().unwrap();
    !status.success()
}
