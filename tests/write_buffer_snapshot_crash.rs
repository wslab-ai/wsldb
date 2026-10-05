#![cfg(all(feature = "crash-injection", unix))]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use support::TestFile;
use wsldb::{Database, DatabaseOptions};

const BUFFER_BYTES: u64 = 64 * 1024 * 1024;

#[test]
fn interrupted_buffer_snapshot_replacement_preserves_the_logical_snapshot() {
    let baseline = TestFile::new("buffer-snapshot-crash-baseline");
    let database = Database::open_with_options(
        baseline.path(),
        DatabaseOptions::new().write_buffer_bytes(BUFFER_BYTES),
    )
    .unwrap();
    for round in 0..4_u32 {
        let mut writer = database.write().unwrap();
        for index in 0..128_u32 {
            writer
                .put(
                    format!("key-{index:04}").as_bytes(),
                    format!("value-{round:02}-{index:04}").as_bytes(),
                )
                .unwrap();
        }
        writer.commit().unwrap();
    }
    let expected: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    drop(database);

    for stage in [
        "write_buffer_snapshot_write",
        "write_buffer_snapshot_sync",
        "write_buffer_snapshot_rename",
        "write_buffer_snapshot_directory_sync",
    ] {
        let target = TestFile::new("buffer-snapshot-crash-target");
        fs::copy(baseline.path(), target.path()).unwrap();
        fs::copy(
            sidecar(baseline.path(), ".wal"),
            sidecar(target.path(), ".wal"),
        )
        .unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("buffered_snapshot_crash_worker")
            .arg("--nocapture")
            .env("WSLDB_BUFFER_SNAPSHOT_PATH", target.path())
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

#[test]
fn buffered_snapshot_crash_worker() {
    let Some(path) = std::env::var_os("WSLDB_BUFFER_SNAPSHOT_PATH") else {
        return;
    };
    Database::open_with_options(
        path,
        DatabaseOptions::new().write_buffer_bytes(BUFFER_BYTES),
    )
    .unwrap()
    .flush()
    .unwrap();
}

fn sidecar(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}
