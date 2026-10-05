mod support;

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

use support::TestFile;
use wsldb::{Database, DatabaseOptions, WriteBatch};

const DEFAULT_ITERATIONS: u64 = 10_000;

#[test]
fn arbitrary_byte_and_truncation_fuzz() {
    if std::env::var_os("WSLDB_RUN_CORRUPTION_FUZZ").is_none() {
        return;
    }
    let iterations = configured_iterations();

    let baseline = TestFile::new("fuzz-baseline");
    let database = Database::open(baseline.path()).unwrap();
    let mut first = database.write().unwrap();
    for index in 0..200_u32 {
        first
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("old-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    first.commit().unwrap();
    let old: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();

    let mut second = database.write().unwrap();
    for index in 0..100_u32 {
        second
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("new-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    second.put(b"overflow", &vec![41; 25_000]).unwrap();
    second.commit().unwrap();
    let newest: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    drop(database);

    let file_len = fs::metadata(baseline.path()).unwrap().len();
    eprintln!("main-file fuzz baseline: {file_len} bytes");
    let mut random = 0xA076_1D64_78BD_642F_u64;
    for iteration in 0..iterations {
        let target = TestFile::new("fuzz-case");
        fs::copy(baseline.path(), target.path()).unwrap();
        random = xorshift(random);
        if random % 4 == 0 {
            OpenOptions::new()
                .write(true)
                .open(target.path())
                .unwrap()
                .set_len(random % file_len)
                .unwrap();
        } else {
            mutate_bytes(target.path(), file_len, random);
        }

        if let Ok(database) = Database::open(target.path())
            && let Ok(entries) = database.read().unwrap().iter()
        {
            let actual = entries.collect::<BTreeMap<_, _>>();
            assert_known_state(
                &actual,
                &[&old, &newest],
                iteration,
                "main file",
                target.path(),
                random,
                file_len,
            );
        }
        if iteration > 0 && iteration % 1_000 == 0 {
            eprintln!("corruption fuzz checkpoint: {iteration}/{iterations}");
        }
    }
}

#[test]
fn buffered_snapshot_and_tail_corruption_fuzz() {
    if std::env::var_os("WSLDB_RUN_CORRUPTION_FUZZ").is_none() {
        return;
    }
    let iterations = configured_iterations();
    let baseline = TestFile::new("wal-fuzz-baseline");
    let direct = Database::open(baseline.path()).unwrap();
    let mut writer = direct.write().unwrap();
    for index in 0..200_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("base-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.commit().unwrap();
    let base: BTreeMap<_, _> = direct.read().unwrap().iter().unwrap().collect();
    drop(direct);

    let database = Database::open_with_options(
        baseline.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    for round in 0..3_u32 {
        let mut writer = database.write().unwrap();
        for index in 0..100_u32 {
            writer
                .put(
                    format!("key-{index:04}").as_bytes(),
                    format!("snapshot-{round}-{index:04}").as_bytes(),
                )
                .unwrap();
        }
        writer
            .put(b"snapshot-overflow", &vec![41 + round as u8; 25_000])
            .unwrap();
        writer.commit().unwrap();
    }
    database.flush().unwrap();
    let snapshot: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();

    let mut first_tail = WriteBatch::new();
    first_tail.put(b"tail-one", b"first").unwrap();
    first_tail.delete(b"key-0007").unwrap();

    let mut second_tail = WriteBatch::new();
    second_tail.put(b"tail-two", b"second").unwrap();
    second_tail.put(b"key-0042", b"tail-update").unwrap();
    database
        .commit_group(vec![first_tail, second_tail])
        .unwrap();
    let newest: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    drop(database);

    let baseline_wal = sidecar(baseline.path(), ".wal");
    let wal_len = fs::metadata(&baseline_wal).unwrap().len();
    eprintln!("write-ahead fuzz baseline: {wal_len} bytes");
    let states = [&base, &snapshot, &newest];
    let mut random = 0xE703_7ED1_A0B4_28DB_u64;
    for iteration in 0..iterations {
        let target = TestFile::new("wal-fuzz-case");
        fs::copy(baseline.path(), target.path()).unwrap();
        let target_wal = sidecar(target.path(), ".wal");
        fs::copy(&baseline_wal, &target_wal).unwrap();
        random = xorshift(random);
        if random % 4 == 0 {
            OpenOptions::new()
                .write(true)
                .open(&target_wal)
                .unwrap()
                .set_len(random % wal_len)
                .unwrap();
        } else {
            mutate_bytes(&target_wal, wal_len, random);
        }

        if let Ok(database) = Database::open(target.path())
            && let Ok(entries) = database.read().unwrap().iter()
        {
            let actual = entries.collect::<BTreeMap<_, _>>();
            assert_known_state(
                &actual,
                &states,
                iteration,
                "write-ahead file",
                &target_wal,
                random,
                wal_len,
            );
        }
        if iteration > 0 && iteration % 1_000 == 0 {
            eprintln!("write-ahead fuzz checkpoint: {iteration}/{iterations}");
        }
    }
}

fn configured_iterations() -> u64 {
    let iterations = std::env::var("WSLDB_CORRUPTION_FUZZ_ITERATIONS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_ITERATIONS);
    assert!(iterations > 0);
    iterations
}

fn assert_known_state(
    actual: &BTreeMap<Vec<u8>, Vec<u8>>,
    expected: &[&BTreeMap<Vec<u8>, Vec<u8>>],
    iteration: u64,
    source: &str,
    mutated_path: &std::path::Path,
    random: u64,
    file_len: u64,
) {
    if expected.contains(&actual) {
        return;
    }
    preserve_failure(mutated_path);
    let distances = expected
        .iter()
        .map(|state| differing_entries(actual, state))
        .collect::<Vec<_>>();
    panic!(
        "silently accepted an unknown logical state from {source} at iteration {iteration}; \
         mutation={}; actual_entries={}; differences={distances:?}",
        mutation_description(random, file_len),
        actual.len(),
    );
}

fn preserve_failure(source: &std::path::Path) {
    let Some(destination) = std::env::var_os("WSLDB_CORRUPTION_FUZZ_FAILURE_PATH") else {
        return;
    };
    fs::copy(source, destination).unwrap();
}

fn differing_entries(
    actual: &BTreeMap<Vec<u8>, Vec<u8>>,
    expected: &BTreeMap<Vec<u8>, Vec<u8>>,
) -> usize {
    let changed = actual
        .iter()
        .filter(|(key, value)| expected.get(key.as_slice()) != Some(*value))
        .count();
    changed
        + expected
            .keys()
            .filter(|key| !actual.contains_key(key.as_slice()))
            .count()
}

fn mutation_description(mut random: u64, file_len: u64) -> String {
    if random % 4 == 0 {
        return format!("truncate_to={}", random % file_len);
    }
    let mut changes = Vec::new();
    for _ in 0..=random % 4 {
        random = xorshift(random);
        changes.push((random % file_len, random % 8));
    }
    format!("bit_flips={changes:?}")
}

fn sidecar(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}

fn mutate_bytes(path: &std::path::Path, file_len: u64, mut random: u64) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    for _ in 0..=random % 4 {
        random = xorshift(random);
        let offset = random % file_len;
        file.seek(SeekFrom::Start(offset)).unwrap();
        let mut byte = [0_u8; 1];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 1 << (random % 8);
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(&byte).unwrap();
    }
    file.sync_all().unwrap();
}

fn xorshift(mut value: u64) -> u64 {
    value ^= value << 13;
    value ^= value >> 7;
    value ^ (value << 17)
}
