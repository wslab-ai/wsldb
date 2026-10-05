mod support;

use std::collections::BTreeMap;
#[cfg(feature = "crash-injection")]
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
#[cfg(feature = "crash-injection")]
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;

use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error};

const PAGE_SIZE: u64 = 4096;

#[test]
fn commit_is_visible_without_changing_an_existing_reader() {
    let file = TestFile::new("snapshot-isolation");
    let database = Database::open(file.path()).unwrap();
    let old_reader = database.read().unwrap();

    let mut writer = database.write().unwrap();
    writer.put(b"key", b"value").unwrap();
    assert_eq!(writer.get(b"key").unwrap(), Some(b"value".to_vec()));
    let committed = writer.commit().unwrap();

    assert_eq!(committed.value(), 1);
    assert_eq!(old_reader.get(b"key").unwrap(), None);
    assert_eq!(
        database.read().unwrap().get(b"key").unwrap(),
        Some(b"value".to_vec())
    );
}

#[test]
fn dropped_writer_aborts_all_changes() {
    let file = TestFile::new("abort");
    let database = Database::open(file.path()).unwrap();

    {
        let mut writer = database.write().unwrap();
        writer.put(b"temporary", b"value").unwrap();
    }

    let reader = database.read().unwrap();
    assert!(reader.is_empty());
    assert_eq!(reader.transaction_id().value(), 0);
}

#[test]
fn committed_data_survives_reopen_in_key_order() {
    let file = TestFile::new("reopen");
    {
        let database = Database::open(file.path()).unwrap();
        let mut writer = database.write().unwrap();
        writer.put(b"beta", b"2").unwrap();
        writer.put(b"alpha", b"1").unwrap();
        writer.commit().unwrap();
    }

    let reopened = Database::open(file.path()).unwrap();
    let reader = reopened.read().unwrap();
    let entries: Vec<_> = reader.iter().unwrap().collect();
    assert_eq!(
        entries,
        vec![
            (b"alpha".to_vec(), b"1".to_vec()),
            (b"beta".to_vec(), b"2".to_vec())
        ]
    );
}

#[test]
fn bounded_page_cache_reports_hits_and_respects_capacity() {
    let file = TestFile::new("page-cache");
    {
        let database = Database::open(file.path()).unwrap();
        let mut writer = database.write().unwrap();
        for index in 0..500_u32 {
            writer
                .put(
                    format!("key-{index:04}").as_bytes(),
                    format!("value-{index:04}").as_bytes(),
                )
                .unwrap();
        }
        writer.commit().unwrap();
    }

    let database =
        Database::open_with_options(file.path(), DatabaseOptions::new().page_cache_capacity(2))
            .unwrap();
    let reader = database.read().unwrap();
    assert_eq!(
        reader.get(b"key-0250").unwrap(),
        Some(b"value-0250".to_vec())
    );
    let after_cold_read = database.page_cache_stats().unwrap();
    assert!(after_cold_read.misses > 0);
    assert!(after_cold_read.resident_pages <= 2);

    assert_eq!(
        reader.get(b"key-0250").unwrap(),
        Some(b"value-0250".to_vec())
    );
    let after_warm_read = database.page_cache_stats().unwrap();
    assert!(after_warm_read.hits > after_cold_read.hits);
    assert!(after_warm_read.resident_pages <= 2);
}

#[test]
fn write_cache_keeps_hot_replacements_without_bulk_leaf_pollution() {
    let file = TestFile::new("selective-write-cache");
    let database = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().page_cache_capacity(1024),
    )
    .unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..500_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}-{}", "x".repeat(80)).as_bytes(),
            )
            .unwrap();
    }
    writer.commit().unwrap();

    let loaded = database.page_cache_stats().unwrap();
    assert!(loaded.resident_pages > 0, "branch pages should be cached");
    assert!(
        loaded.resident_pages + 1 < database.stats().unwrap().last_page as usize,
        "bulk-created leaf pages should remain cold"
    );

    assert!(database.read().unwrap().get(b"key-0250").unwrap().is_some());
    let after_cold_read = database.page_cache_stats().unwrap();
    assert!(after_cold_read.misses > loaded.misses);

    let mut writer = database.write().unwrap();
    writer.put(b"key-0250", b"replacement").unwrap();
    writer.commit().unwrap();
    let after_replacement = database.page_cache_stats().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"key-0250").unwrap(),
        Some(b"replacement".to_vec())
    );
    let after_hot_read = database.page_cache_stats().unwrap();
    assert_eq!(after_hot_read.misses, after_replacement.misses);
    assert!(after_hot_read.hits > after_replacement.hits);
}

#[test]
fn delete_is_durable() {
    let file = TestFile::new("delete");
    let database = Database::open(file.path()).unwrap();

    let mut first = database.write().unwrap();
    first.put(b"key", b"value").unwrap();
    first.commit().unwrap();

    let mut second = database.write().unwrap();
    assert!(second.delete(b"key").unwrap());
    second.commit().unwrap();
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.read().unwrap().get(b"key").unwrap(), None);
}

#[test]
fn overflow_value_survives_reopen_and_is_checksummed() {
    let file = TestFile::new("overflow");
    let value: Vec<u8> = (0..25_000_u32).map(|index| (index % 251) as u8).collect();
    {
        let database = Database::open(file.path()).unwrap();
        let mut writer = database.write().unwrap();
        writer.put(b"large", &value).unwrap();
        writer.commit().unwrap();
    }

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.read().unwrap().get(b"large").unwrap(), Some(value));
    drop(reopened);

    flip_byte(file.path(), 2 * PAGE_SIZE);
    let reopened = Database::open(file.path()).unwrap();
    assert!(matches!(reopened.verify(), Err(Error::Corrupt(_))));
    assert!(matches!(
        reopened.read().unwrap().get(b"large"),
        Err(Error::Corrupt(_))
    ));
    let fallback =
        Database::open_with_options(file.path(), DatabaseOptions::new().verify_on_open(true))
            .unwrap();
    assert_eq!(fallback.stats().unwrap().transaction_id.value(), 0);
    assert!(fallback.read().unwrap().is_empty());
}

#[test]
fn full_verification_bypasses_cached_pages() {
    let file = TestFile::new("uncached-verify");
    let database = Database::open(file.path()).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"value").unwrap();
    writer.commit().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"key").unwrap(),
        Some(b"value".to_vec())
    );

    let root = read_u64_at(file.path(), PAGE_SIZE + 24);
    flip_byte(file.path(), root * PAGE_SIZE);
    assert!(matches!(database.verify(), Err(Error::Corrupt(_))));
}

#[test]
fn many_keys_split_across_multiple_tree_levels() {
    let file = TestFile::new("tree-splits");
    {
        let database = Database::open(file.path()).unwrap();
        let mut writer = database.write().unwrap();
        for index in 0..3000_u32 {
            let key = format!("key-{index:06}");
            let value = format!("value-{index:06}-{}", "x".repeat(80));
            writer.put(key.as_bytes(), value.as_bytes()).unwrap();
        }
        writer.commit().unwrap();
        assert!(database.stats().unwrap().last_page > 2);
    }

    let reopened = Database::open(file.path()).unwrap();
    let report = reopened.verify().unwrap();
    assert_eq!(report.entries, 3000);
    assert!(report.tree_pages > 1);
    assert_eq!(report.overflow_pages, 0);
    assert!(report.tree_depth > 1);
    assert_eq!(
        reopened.stats().unwrap().last_page,
        report.tree_pages + report.overflow_pages + 1
    );
    let reader = reopened.read().unwrap();
    assert_eq!(reader.len(), 3000);
    for index in [0_u32, 1, 37, 1024, 2048, 2999] {
        let key = format!("key-{index:06}");
        let expected = format!("value-{index:06}-{}", "x".repeat(80)).into_bytes();
        assert_eq!(reader.get(key.as_bytes()).unwrap(), Some(expected));
    }
}

#[test]
fn widespread_deletes_rebalance_sparse_pages() {
    const ENTRIES: u32 = 6_000;
    let file = TestFile::new("delete-rebalance");
    let database = Database::open(file.path()).unwrap();
    let value = vec![b'x'; 256];
    let mut insert = database.write().unwrap();
    for index in 0..ENTRIES {
        insert
            .put(format!("key-{index:06}").as_bytes(), &value)
            .unwrap();
    }
    insert.commit().unwrap();
    let before = database.verify().unwrap();
    assert!(before.tree_depth >= 3);

    let mut delete = database.write().unwrap();
    for index in 0..ENTRIES {
        if index % 10 != 0 {
            assert!(delete.delete(format!("key-{index:06}").as_bytes()).unwrap());
        }
    }
    delete.commit().unwrap();

    let after = database.verify().unwrap();
    assert_eq!(after.entries, u64::from(ENTRIES / 10));
    assert!(after.tree_pages * 4 < before.tree_pages);
    assert!(after.tree_depth <= before.tree_depth);
    let reader = database.read().unwrap();
    for index in (0..ENTRIES).step_by(10) {
        assert_eq!(
            reader.get(format!("key-{index:06}").as_bytes()).unwrap(),
            Some(value.clone())
        );
    }
}

#[test]
fn range_scan_obeys_bounds_limit_and_order() {
    let file = TestFile::new("range-scan");
    let database = Database::open(file.path()).unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..300_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.commit().unwrap();

    let reader = database.read().unwrap();
    let entries: Vec<_> = reader
        .range(b"key-0050", Some(b"key-0060"), 4)
        .unwrap()
        .collect();
    assert_eq!(
        entries,
        (50..54_u32)
            .map(|index| {
                (
                    format!("key-{index:04}").into_bytes(),
                    format!("value-{index:04}").into_bytes(),
                )
            })
            .collect::<Vec<_>>()
    );

    let after_gap: Vec<_> = reader
        .range(b"key-0050-extra", Some(b"key-0053"), 10)
        .unwrap()
        .collect();
    assert_eq!(
        after_gap
            .iter()
            .map(|(key, _)| key.as_slice())
            .collect::<Vec<_>>(),
        vec![b"key-0051", b"key-0052"]
    );
    assert_eq!(reader.range(b"z", Some(b"a"), 10).unwrap().len(), 0);
    assert_eq!(reader.range(b"", None, 0).unwrap().len(), 0);
}

#[test]
fn mixed_mutations_match_an_independent_ordered_model() {
    let file = TestFile::new("model");
    let database = Database::open(file.path()).unwrap();
    let mut model = BTreeMap::new();
    let mut random = 0x4D59_5DF4_D0F3_3173_u64;

    for _round in 0..16 {
        let mut writer = database.write().unwrap();
        for _operation in 0..80 {
            random = xorshift(random);
            let key = format!("key-{:04}", random % 700).into_bytes();
            if random % 4 == 0 {
                let expected = model.remove(&key).is_some();
                assert_eq!(writer.delete(&key).unwrap(), expected);
            } else {
                let value = format!("value-{random:016x}").into_bytes();
                writer.put(&key, &value).unwrap();
                model.insert(key, value);
            }
        }
        writer.commit().unwrap();

        let actual: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
        assert_eq!(actual, model);
    }
}

#[test]
fn stale_database_handle_gets_a_write_conflict() {
    let file = TestFile::new("write-conflict");
    let first_database = Database::open(file.path()).unwrap();
    let second_database = Database::open(file.path()).unwrap();
    let mut first = first_database.write().unwrap();
    let mut stale = second_database.write().unwrap();
    first.put(b"first", b"value").unwrap();
    stale.put(b"stale", b"value").unwrap();

    first.commit().unwrap();
    assert!(matches!(stale.commit(), Err(Error::WriteConflict)));
}

#[test]
fn concurrent_handles_serialize_commits_and_reject_one_stale_writer() {
    let file = TestFile::new("concurrent-commit");
    let first_database = Database::open(file.path()).unwrap();
    let second_database = Database::open(file.path()).unwrap();
    let barrier = Arc::new(Barrier::new(2));

    let first_barrier = Arc::clone(&barrier);
    let first = thread::spawn(move || {
        let mut writer = first_database.write().unwrap();
        writer.put(b"first", b"value").unwrap();
        first_barrier.wait();
        writer.commit()
    });
    let second = thread::spawn(move || {
        let mut writer = second_database.write().unwrap();
        writer.put(b"second", b"value").unwrap();
        barrier.wait();
        writer.commit()
    });

    let outcomes = [first.join().unwrap(), second.join().unwrap()];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| matches!(result, Err(Error::WriteConflict)))
            .count(),
        1
    );

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 1);
}

#[test]
fn corrupt_newest_metadata_falls_back_to_previous_commit() {
    let file = TestFile::new("metadata-fallback");
    commit_two_versions(file.path());

    flip_byte(file.path(), 24);

    let recovered = Database::open(file.path()).unwrap();
    let reader = recovered.read().unwrap();
    assert_eq!(reader.transaction_id().value(), 1);
    assert_eq!(reader.get(b"key").unwrap(), Some(b"one".to_vec()));
}

#[test]
fn corrupt_newest_root_page_falls_back_to_previous_commit() {
    let file = TestFile::new("root-fallback");
    commit_two_versions(file.path());
    let newest_root = read_u64_at(file.path(), 24);
    assert!(newest_root >= 2);

    flip_byte(file.path(), newest_root * PAGE_SIZE);

    let recovered = Database::open(file.path()).unwrap();
    let reader = recovered.read().unwrap();
    assert_eq!(reader.transaction_id().value(), 1);
    assert_eq!(reader.get(b"key").unwrap(), Some(b"one".to_vec()));
}

#[test]
fn open_fails_when_both_metadata_slots_are_corrupt() {
    let file = TestFile::new("fail-closed");
    {
        let database = Database::open(file.path()).unwrap();
        let mut writer = database.write().unwrap();
        writer.put(b"key", b"value").unwrap();
        writer.commit().unwrap();
    }

    flip_byte(file.path(), 0);
    flip_byte(file.path(), PAGE_SIZE);

    assert!(matches!(
        Database::open(file.path()),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn open_rejects_an_existing_empty_database_file() {
    let file = TestFile::new("empty-existing-file");
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(file.path())
        .unwrap();

    assert!(matches!(
        Database::open(file.path()),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(std::fs::metadata(file.path()).unwrap().len(), 0);
}

#[test]
fn empty_keys_are_rejected_before_mutation() {
    let file = TestFile::new("invalid-entry");
    let database = Database::open(file.path()).unwrap();
    let mut writer = database.write().unwrap();

    assert!(matches!(
        writer.put(b"", b"value"),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(writer.get(b"key").unwrap(), None);
}

#[cfg(feature = "crash-injection")]
#[test]
fn interrupted_commits_recover_to_one_complete_snapshot() {
    let baseline = TestFile::new("crash-baseline");
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
    let old: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    let expected = expected_after_crash_mutations(old.clone());
    drop(database);

    let mut reached_end_of_page_writes = false;
    for failure_after in 1..=128_u64 {
        let crashed = run_interrupted_commit(
            baseline.path(),
            &old,
            &expected,
            "page_write",
            failure_after,
        );
        if !crashed {
            reached_end_of_page_writes = true;
            break;
        }
    }
    assert!(reached_end_of_page_writes);

    for stage in ["data_sync", "metadata_write", "metadata_sync"] {
        assert!(run_interrupted_commit(
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
fn crash_commit_worker() {
    let Some(path) = std::env::var_os("WSLDB_CRASH_WORKER_PATH") else {
        return;
    };
    let database = Database::open(path).unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..16_u32 {
        writer.delete(format!("key-{index:04}").as_bytes()).unwrap();
    }
    for index in 16..32_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("new-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    for index in 64..128_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("new-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.put(b"large", &crash_large_value()).unwrap();
    writer.commit().unwrap();
}

#[cfg(feature = "crash-injection")]
fn run_interrupted_commit(
    baseline: &Path,
    old: &BTreeMap<Vec<u8>, Vec<u8>>,
    expected: &BTreeMap<Vec<u8>, Vec<u8>>,
    stage: &str,
    failure_after: u64,
) -> bool {
    let target = TestFile::new("crash-target");
    fs::copy(baseline, target.path()).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("crash_commit_worker")
        .arg("--nocapture")
        .env("WSLDB_CRASH_WORKER_PATH", target.path())
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
    assert!(
        actual == *old || actual == *expected,
        "recovered a mixed snapshot after {stage} hit {failure_after}"
    );
    !status.success()
}

#[cfg(feature = "crash-injection")]
fn expected_after_crash_mutations(
    mut entries: BTreeMap<Vec<u8>, Vec<u8>>,
) -> BTreeMap<Vec<u8>, Vec<u8>> {
    for index in 0..16_u32 {
        entries.remove(format!("key-{index:04}").as_bytes());
    }
    for index in 16..32_u32 {
        entries.insert(
            format!("key-{index:04}").into_bytes(),
            format!("new-{index:04}").into_bytes(),
        );
    }
    for index in 64..128_u32 {
        entries.insert(
            format!("key-{index:04}").into_bytes(),
            format!("new-{index:04}").into_bytes(),
        );
    }
    entries.insert(b"large".to_vec(), crash_large_value());
    entries
}

#[cfg(feature = "crash-injection")]
fn crash_large_value() -> Vec<u8> {
    (0..25_000_u32).map(|index| (index % 251) as u8).collect()
}

fn commit_two_versions(path: &Path) {
    let database = Database::open(path).unwrap();
    let mut first = database.write().unwrap();
    first.put(b"key", b"one").unwrap();
    first.commit().unwrap();

    let mut second = database.write().unwrap();
    second.put(b"key", b"two").unwrap();
    second.commit().unwrap();
}

fn xorshift(mut value: u64) -> u64 {
    value ^= value << 13;
    value ^= value >> 7;
    value ^ (value << 17)
}

fn read_u64_at(path: &Path, offset: u64) -> u64 {
    let mut file = OpenOptions::new().read(true).open(path).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut bytes = [0_u8; 8];
    file.read_exact(&mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

fn flip_byte(path: &Path, offset: u64) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 0xFF;
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&byte).unwrap();
    file.sync_all().unwrap();
}
