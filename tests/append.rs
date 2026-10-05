mod support;

#[cfg(feature = "crash-injection")]
use std::collections::BTreeMap;
#[cfg(feature = "crash-injection")]
use std::fs;
#[cfg(feature = "crash-injection")]
use std::path::Path;
#[cfg(feature = "crash-injection")]
use std::process::{Command, Stdio};

use support::TestFile;
use wsldb::Database;

#[test]
fn sequential_batches_preserve_a_verified_tree() {
    let file = TestFile::new("append-batches");
    let database = Database::open(file.path()).unwrap();
    for batch in 0..12_u32 {
        let mut writer = database.write().unwrap();
        for index in batch * 500..(batch + 1) * 500 {
            writer
                .put(
                    format!("key-{index:08}").as_bytes(),
                    format!("value-{index:08}").as_bytes(),
                )
                .unwrap();
        }
        writer.commit().unwrap();
    }

    let report = database.verify().unwrap();
    assert_eq!(report.entries, 6_000);
    assert!(report.tree_depth >= 2);
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    let reader = reopened.read().unwrap();
    for index in [0_u32, 499, 500, 2_999, 5_999] {
        assert_eq!(
            reader.get(format!("key-{index:08}").as_bytes()).unwrap(),
            Some(format!("value-{index:08}").into_bytes())
        );
    }
}

#[cfg(feature = "crash-injection")]
#[test]
fn interrupted_append_recovers_one_complete_snapshot() {
    let baseline = TestFile::new("append-crash-baseline");
    let database = Database::open(baseline.path()).unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..128_u32 {
        put_entry(&mut writer, index);
    }
    writer.commit().unwrap();
    let old: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    let mut expected = old.clone();
    for index in 128..768_u32 {
        expected.insert(key(index), value(index));
    }
    drop(database);

    let mut reached_end = false;
    for failure_after in 1..=64_u64 {
        if !run_interrupted_append(
            baseline.path(),
            &old,
            &expected,
            "page_write",
            failure_after,
        ) {
            reached_end = true;
            break;
        }
    }
    assert!(reached_end);
    for stage in ["data_sync", "metadata_write", "metadata_sync"] {
        assert!(run_interrupted_append(
            baseline.path(),
            &old,
            &expected,
            stage,
            1,
        ));
    }
}

#[cfg(feature = "crash-injection")]
#[test]
fn append_crash_worker() {
    let Some(path) = std::env::var_os("WSLDB_APPEND_CRASH_PATH") else {
        return;
    };
    let database = Database::open(path).unwrap();
    let mut writer = database.write().unwrap();
    for index in 128..768_u32 {
        put_entry(&mut writer, index);
    }
    writer.commit().unwrap();
}

#[cfg(feature = "crash-injection")]
fn run_interrupted_append(
    baseline: &Path,
    old: &BTreeMap<Vec<u8>, Vec<u8>>,
    expected: &BTreeMap<Vec<u8>, Vec<u8>>,
    stage: &str,
    failure_after: u64,
) -> bool {
    let target = TestFile::new("append-crash-target");
    fs::copy(baseline, target.path()).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("append_crash_worker")
        .arg("--nocapture")
        .env("WSLDB_APPEND_CRASH_PATH", target.path())
        .env("WSLDB_FAILPOINT", stage)
        .env("WSLDB_FAIL_AFTER", failure_after.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();

    let recovered = Database::open(target.path()).unwrap();
    recovered.verify().unwrap();
    let actual: BTreeMap<_, _> = recovered.read().unwrap().iter().unwrap().collect();
    assert!(actual == *old || actual == *expected);
    !status.success()
}

#[cfg(feature = "crash-injection")]
fn put_entry(writer: &mut wsldb::WriteTransaction, index: u32) {
    writer.put(&key(index), &value(index)).unwrap();
}

#[cfg(feature = "crash-injection")]
fn key(index: u32) -> Vec<u8> {
    format!("key-{index:08}").into_bytes()
}

#[cfg(feature = "crash-injection")]
fn value(index: u32) -> Vec<u8> {
    format!("value-{index:08}").into_bytes()
}
