mod support;

use std::path::Path;
use std::process::{Command, Stdio};

use support::TestFile;
use wsldb::Database;
#[cfg(feature = "fault-injection")]
use wsldb::Error;

#[test]
fn creation_atomically_publishes_a_reopenable_database() {
    let file = TestFile::new("atomic-initialization");
    let database = Database::open(file.path()).unwrap();
    assert_eq!(database.verify().unwrap().entries, 0);
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 0);
}

#[test]
fn concurrent_processes_initialize_one_database() {
    let file = TestFile::new("concurrent-initialization");
    let children: Vec<_> = (0..8)
        .map(|_| {
            Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("initialization_open_worker")
                .env("WSLDB_INITIALIZATION_OPEN_PATH", file.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    assert_valid_empty_database(file.path());
}

#[test]
fn initialization_open_worker() {
    let Some(path) = std::env::var_os("WSLDB_INITIALIZATION_OPEN_PATH") else {
        return;
    };
    assert_valid_empty_database(Path::new(&path));
}

#[cfg(feature = "fault-injection")]
#[test]
fn initialization_failures_distinguish_pre_and_post_publication() {
    let file = TestFile::new("initialization-partial-write");
    run_worker(file.path(), "fault-definite", "positioned_write", Some(1));
    assert!(!file.path().exists(), "partial metadata was published");

    for stage in ["initial_metadata_sync", "snapshot_publish_link"] {
        let file = TestFile::new("initialization-definite-failure");
        run_worker(file.path(), "fault-definite", stage, None);
        assert!(!file.path().exists(), "target published at {stage}");
    }

    for stage in [
        "snapshot_publish_directory_sync",
        "snapshot_temporary_remove",
        "snapshot_cleanup_directory_sync",
    ] {
        let file = TestFile::new("initialization-ambiguous-failure");
        run_worker(file.path(), "fault-ambiguous", stage, None);
        assert_valid_empty_database(file.path());
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn initialization_fault_worker() {
    let Some(path) = std::env::var_os("WSLDB_INITIALIZATION_WORKER_PATH") else {
        return;
    };
    let mode = std::env::var("WSLDB_INITIALIZATION_WORKER_MODE").unwrap();
    let error = match Database::open(Path::new(&path)) {
        Ok(_) => panic!("initialization unexpectedly succeeded"),
        Err(error) => error,
    };
    match mode.as_str() {
        "fault-definite" => assert!(matches!(&error, Error::Io(_)), "got {error}"),
        "fault-ambiguous" => assert!(
            matches!(&error, Error::PublicationOutcomeUnknown(_)),
            "got {error}"
        ),
        _ => panic!("unexpected initialization worker mode: {mode}"),
    }
}

#[cfg(feature = "crash-injection")]
#[test]
fn interrupted_initialization_never_publishes_partial_metadata() {
    for stage in [
        "initial_metadata_write",
        "initial_metadata_sync",
        "snapshot_publish_link",
        "snapshot_publish_directory_sync",
        "snapshot_temporary_remove",
        "snapshot_cleanup_directory_sync",
    ] {
        let file = TestFile::new("initialization-crash");
        run_crash_worker(file.path(), stage);
        let database = Database::open(file.path()).unwrap();
        assert_eq!(database.verify().unwrap().entries, 0, "stage {stage}");
    }
}

#[cfg(feature = "crash-injection")]
#[test]
fn initialization_crash_worker() {
    let Some(path) = std::env::var_os("WSLDB_INITIALIZATION_WORKER_PATH") else {
        return;
    };
    let _database = Database::open(Path::new(&path)).unwrap();
    panic!("configured initialization crash hook was not reached");
}

#[cfg(any(feature = "crash-injection", feature = "fault-injection"))]
fn worker_command(path: &Path, mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("--exact")
        .arg(if mode == "crash" {
            "initialization_crash_worker"
        } else {
            "initialization_fault_worker"
        })
        .arg("--nocapture")
        .env("WSLDB_INITIALIZATION_WORKER_PATH", path)
        .env("WSLDB_INITIALIZATION_WORKER_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(feature = "fault-injection")]
fn run_worker(path: &Path, mode: &str, stage: &str, fail_after_bytes: Option<u64>) {
    let mut command = worker_command(path, mode);
    command.env("WSLDB_IO_FAILPOINT", stage);
    if let Some(bytes) = fail_after_bytes {
        command.env("WSLDB_IO_FAIL_AFTER_BYTES", bytes.to_string());
    }
    let status = command.status().unwrap();
    assert!(status.success(), "worker failed at {stage}");
}

#[cfg(feature = "crash-injection")]
fn run_crash_worker(path: &Path, stage: &str) {
    let status = worker_command(path, "crash")
        .env("WSLDB_FAILPOINT", stage)
        .status()
        .unwrap();
    assert!(!status.success(), "worker did not crash at {stage}");
}

fn assert_valid_empty_database(path: &Path) {
    let database = Database::open(path).unwrap();
    assert_eq!(database.verify().unwrap().entries, 0);
}
