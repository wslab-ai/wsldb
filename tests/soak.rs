mod support;

use std::collections::BTreeMap;

use support::TestFile;
use wsldb::Database;

const DEFAULT_OPERATIONS: u64 = 10_000_000;
const BATCH_SIZE: u64 = 1_000;
const KEY_SPACE: u64 = 100_000;
const CHECKPOINT_OPERATIONS: u64 = 100_000;
const DEFAULT_COMPACTION_THRESHOLD_BYTES: u64 = 512 * 1024 * 1024;

#[test]
fn model_based_ten_million_operation_soak() {
    if std::env::var_os("WSLDB_RUN_SOAK").is_none() {
        return;
    }
    let operations = std::env::var("WSLDB_SOAK_OPERATIONS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_OPERATIONS);
    let compaction_threshold = std::env::var("WSLDB_SOAK_COMPACT_BYTES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_COMPACTION_THRESHOLD_BYTES);
    assert!(operations > 0);

    let file = TestFile::new("model-soak");
    let mut database = Database::open(file.path()).unwrap();
    let mut model = BTreeMap::new();
    let mut random = 0xD1B5_4A32_D192_ED03_u64;
    let mut completed = 0_u64;

    while completed < operations {
        let end = completed.saturating_add(BATCH_SIZE).min(operations);
        let mut writer = database.write().unwrap();
        while completed < end {
            random = xorshift(random);
            let key = format!("key-{:06}", random % KEY_SPACE).into_bytes();
            if random % 5 == 0 {
                let expected = model.remove(&key).is_some();
                assert_eq!(writer.delete(&key).unwrap(), expected);
            } else {
                let value = deterministic_value(random, completed);
                writer.put(&key, &value).unwrap();
                model.insert(key, value);
            }
            completed += 1;
        }
        writer.commit().unwrap();

        if completed % CHECKPOINT_OPERATIONS == 0 || completed == operations {
            let stats = database.operational_stats().unwrap();
            assert_eq!(stats.snapshot.entries, model.len() as u64);
            compare_sample(&database, &model, completed);
            compare_ranges(&database, &model, completed);
            eprintln!(
                "soak checkpoint: {completed}/{operations}; file={} MiB; live={} MiB; retired={} MiB",
                stats.file_bytes / (1024 * 1024),
                stats.live_bytes / (1024 * 1024),
                stats.retired_bytes / (1024 * 1024),
            );
            drop(database);
            if stats.retired_bytes >= compaction_threshold {
                let report = Database::compact_file(file.path()).unwrap();
                eprintln!(
                    "soak compaction: {} MiB -> {} MiB; transaction={}",
                    report.source_bytes / (1024 * 1024),
                    report.compacted_bytes / (1024 * 1024),
                    report.compacted_snapshot.transaction_id.value(),
                );
            }
            database = Database::open(file.path()).unwrap();
        }
    }

    let actual: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    assert_eq!(actual, model);
}

fn compare_sample(database: &Database, model: &BTreeMap<Vec<u8>, Vec<u8>>, seed: u64) {
    let reader = database.read().unwrap();
    let mut random = seed ^ 0x94D0_49BB_1331_11EB;
    for _ in 0..1_000 {
        random = xorshift(random);
        let key = format!("key-{:06}", random % KEY_SPACE).into_bytes();
        assert_eq!(reader.get(&key).unwrap(), model.get(&key).cloned());
    }
}

fn compare_ranges(database: &Database, model: &BTreeMap<Vec<u8>, Vec<u8>>, seed: u64) {
    let reader = database.read().unwrap();
    let mut random = seed ^ 0x2545_F491_4F6C_DD1D;
    for query in 0..100_u64 {
        random = xorshift(random);
        let start_index = random % KEY_SPACE;
        let start = format!("key-{start_index:06}").into_bytes();
        random = xorshift(random);
        let limit = usize::try_from(random % 129).unwrap();
        let end = if query % 4 == 0 {
            None
        } else {
            let end_index = start_index
                .saturating_add(1 + random % 2_048)
                .min(KEY_SPACE);
            Some(format!("key-{end_index:06}").into_bytes())
        };

        let actual: Vec<_> = reader
            .range(&start, end.as_deref(), limit)
            .unwrap()
            .collect();
        let expected: Vec<_> = match end {
            Some(end) => model
                .range(start..end)
                .take(limit)
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            None => model
                .range(start..)
                .take(limit)
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        };
        assert_eq!(actual, expected);
    }
}

fn deterministic_value(random: u64, operation: u64) -> Vec<u8> {
    let len = if operation % 10_000 == 0 { 3_000 } else { 64 };
    let mut value = Vec::with_capacity(len);
    let mut state = random ^ operation;
    for _ in 0..len {
        state = xorshift(state);
        value.push(state as u8);
    }
    value
}

fn xorshift(mut value: u64) -> u64 {
    value ^= value << 13;
    value ^= value >> 7;
    value ^ (value << 17)
}
