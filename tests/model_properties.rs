mod support;

use std::collections::BTreeMap;

use support::TestFile;
use wsldb::{Database, DatabaseOptions, ReadTransaction};

const ROUNDS: u32 = 12;
const OPERATIONS_PER_ROUND: u32 = 48;
const KEY_SPACE: u64 = 160;

#[test]
fn direct_and_buffered_engines_preserve_model_across_snapshots_ranges_and_reopen() {
    run_model("direct", 0);
    run_model("buffered", 16 * 1024);
}

fn run_model(label: &str, write_buffer_bytes: u64) {
    let file = TestFile::new(label);
    let options = DatabaseOptions::new().write_buffer_bytes(write_buffer_bytes);
    let database = Database::open_with_options(file.path(), options).unwrap();
    let mut model = BTreeMap::new();
    let mut random = 0xA076_1D64_78BD_642F_u64 ^ write_buffer_bytes;

    for round in 0..ROUNDS {
        let previous_model = model.clone();
        let previous_snapshot = database.read().unwrap();
        assert_snapshot(&previous_snapshot, &previous_model);

        let mut writer = database.write().unwrap();
        for operation in 0..OPERATIONS_PER_ROUND {
            random = xorshift(random);
            let key = format!("key-{:04}", random % KEY_SPACE).into_bytes();
            if random % 5 == 0 {
                let expected = model.remove(&key).is_some();
                assert_eq!(writer.delete(&key).unwrap(), expected);
            } else {
                let value = deterministic_value(random, round, operation);
                writer.put(&key, &value).unwrap();
                model.insert(key.clone(), value);
            }
            assert_eq!(writer.get(&key).unwrap(), model.get(&key).cloned());
        }
        writer.commit().unwrap();

        assert_snapshot(&previous_snapshot, &previous_model);
        let current_snapshot = database.read().unwrap();
        assert_snapshot(&current_snapshot, &model);
        assert_random_ranges(&current_snapshot, &model, &mut random);

        if write_buffer_bytes > 0 && round == ROUNDS / 2 {
            database.flush().unwrap();
            assert_snapshot(&previous_snapshot, &previous_model);
            assert_snapshot(&database.read().unwrap(), &model);
        }
    }

    if write_buffer_bytes > 0 {
        database.flush().unwrap();
    }
    assert_eq!(database.verify().unwrap().entries, model.len() as u64);
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    let recovered = reopened.read().unwrap();
    assert_snapshot(&recovered, &model);
    assert_random_ranges(&recovered, &model, &mut random);
}

fn assert_snapshot(snapshot: &ReadTransaction, model: &BTreeMap<Vec<u8>, Vec<u8>>) {
    assert_eq!(snapshot.len(), model.len() as u64);
    assert_eq!(snapshot.iter().unwrap().collect::<BTreeMap<_, _>>(), *model);
    assert_paginated_scan(snapshot, model);
}

fn assert_paginated_scan(snapshot: &ReadTransaction, model: &BTreeMap<Vec<u8>, Vec<u8>>) {
    const PAGE_ENTRIES: usize = 7;

    let mut actual = Vec::new();
    let mut after: Option<Vec<u8>> = None;
    loop {
        let page = snapshot
            .range_after(after.as_deref(), None, PAGE_ENTRIES)
            .unwrap()
            .collect::<Vec<_>>();
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= PAGE_ENTRIES);
        after = page.last().map(|(key, _)| key.clone());
        actual.extend(page);
    }
    assert_eq!(actual, model.clone().into_iter().collect::<Vec<_>>());

    let after = b"key-0040".as_slice();
    let end = b"key-0080".as_slice();
    let bounded = snapshot
        .range_after(Some(after), Some(end), PAGE_ENTRIES)
        .unwrap()
        .collect::<Vec<_>>();
    let expected = model
        .range::<[u8], _>((
            std::ops::Bound::Excluded(after),
            std::ops::Bound::Excluded(end),
        ))
        .take(PAGE_ENTRIES)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Vec<_>>();
    assert_eq!(bounded, expected);
    assert_eq!(snapshot.range_after(None, None, 0).unwrap().len(), 0);
}

fn assert_random_ranges(
    snapshot: &ReadTransaction,
    model: &BTreeMap<Vec<u8>, Vec<u8>>,
    random: &mut u64,
) {
    for query in 0..12_u32 {
        *random = xorshift(*random);
        let start = format!("key-{:04}", *random % KEY_SPACE).into_bytes();
        *random = xorshift(*random);
        let end =
            (query % 4 != 0).then(|| format!("key-{:04}", *random % (KEY_SPACE + 1)).into_bytes());
        *random = xorshift(*random);
        let limit = usize::try_from(*random % 18).unwrap();

        let expected = model
            .iter()
            .filter(|(key, _)| {
                key.as_slice() >= start.as_slice()
                    && end
                        .as_ref()
                        .is_none_or(|end| key.as_slice() < end.as_slice())
            })
            .take(limit)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        let actual = snapshot
            .range(&start, end.as_deref(), limit)
            .unwrap()
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}

fn deterministic_value(random: u64, round: u32, operation: u32) -> Vec<u8> {
    let length = if (round + operation).is_multiple_of(29) {
        3_000
    } else {
        usize::try_from(random % 96).unwrap()
    };
    let mut value = Vec::with_capacity(length);
    let mut state = random ^ u64::from(round) << 32 ^ u64::from(operation);
    for _ in 0..length {
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
