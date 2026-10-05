mod support;

use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use support::TestFile;
use wsldb::Database;
#[cfg(unix)]
use wsldb::DatabaseOptions;

const MAIN_FIXTURE: &[u8] = include_bytes!("fixtures/database.wsldb");
const BUFFERED_FIXTURE: &[u8] = include_bytes!("fixtures/buffered.wsldb");
const BUFFERED_WAL_FIXTURE: &[u8] = include_bytes!("fixtures/buffered.wsldb.wal");

#[test]
fn committed_format_fixtures_remain_readable() {
    if let Some(directory) = std::env::var_os("WSLDB_WRITE_FORMAT_FIXTURES") {
        generate_fixtures(Path::new(&directory));
        return;
    }

    verify_main_fixture();
    verify_buffered_fixture();
}

#[test]
fn committed_fixture_envelopes_match_the_documented_layout() {
    assert_metadata(MAIN_FIXTURE, 0, 2, 22, 22, 299);
    assert_metadata(MAIN_FIXTURE, 1, 1, 17, 17, 301);
    assert_metadata(BUFFERED_FIXTURE, 0, 0, 0, 1, 0);
    assert!(BUFFERED_FIXTURE[4096..8192].iter().all(|byte| *byte == 0));

    let mut offset = 0;
    offset = assert_wal_record(BUFFERED_WAL_FIXTURE, offset, b"WSLDBSNP", 0, 2, 3, 4);
    offset = assert_wal_record(BUFFERED_WAL_FIXTURE, offset, b"WSLDBWAL", 2, 3, 3, 1);
    offset = assert_wal_record(BUFFERED_WAL_FIXTURE, offset, b"WSLDBWAL", 3, 4, 4, 1);
    assert_eq!(offset, BUFFERED_WAL_FIXTURE.len());
}

fn assert_metadata(
    bytes: &[u8],
    slot: usize,
    transaction: u64,
    root: u64,
    last_page: u64,
    entries: u64,
) {
    let offset = slot * 4096;
    let page = &bytes[offset..offset + 4096];
    assert_eq!(&page[..8], b"WSLDB\0\0\0");
    assert_eq!(&page[8..12], &[0; 4]);
    assert_eq!(read_u32(&page[12..16]), 4096);
    assert_eq!(read_u64(&page[16..24]), transaction);
    assert_eq!(read_u64(&page[24..32]), root);
    assert_eq!(read_u64(&page[32..40]), last_page);
    assert_eq!(read_u64(&page[40..48]), entries);
}

fn assert_wal_record(
    bytes: &[u8],
    offset: usize,
    magic: &[u8; 8],
    base: u64,
    transaction: u64,
    entries: u64,
    items: u32,
) -> usize {
    let header = &bytes[offset..offset + 52];
    assert_eq!(&header[..8], magic);
    let len = usize::try_from(read_u64(&header[8..16])).unwrap();
    assert!(len >= 68);
    assert!(offset + len <= bytes.len());
    assert_eq!(read_u64(&header[16..24]), base);
    assert_eq!(read_u64(&header[24..32]), transaction);
    assert_eq!(read_u64(&header[32..40]), entries);
    assert_eq!(read_u32(&header[40..44]), items);
    let footer = &bytes[offset + len - 16..offset + len];
    assert_eq!(&footer[4..8], b"WEND");
    assert_eq!(usize::try_from(read_u64(&footer[8..16])).unwrap(), len);
    offset + len
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().unwrap())
}

fn verify_main_fixture() {
    let target = TestFile::new("main-format-fixture");
    fs::write(target.path(), MAIN_FIXTURE).unwrap();
    let database = Database::open(target.path()).unwrap();
    assert_eq!(database.stats().unwrap().transaction_id.value(), 2);
    let verified = assert_database(&database, &main_entries());
    assert!(verified.tree_depth >= 2);
    assert!(verified.overflow_pages > 0);
}

fn verify_buffered_fixture() {
    let target = TestFile::new("buffered-format-fixture");
    fs::write(target.path(), BUFFERED_FIXTURE).unwrap();
    fs::write(sidecar(target.path(), ".wal"), BUFFERED_WAL_FIXTURE).unwrap();
    let database = Database::open(target.path()).unwrap();
    assert_eq!(database.stats().unwrap().transaction_id.value(), 4);
    assert_database(&database, &buffered_entries());
}

fn assert_database(
    database: &Database,
    expected: &BTreeMap<Vec<u8>, Vec<u8>>,
) -> wsldb::VerificationReport {
    let actual: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    assert_eq!(&actual, expected);
    let verified = database.verify().unwrap();
    assert_eq!(verified.entries, expected.len() as u64);
    verified
}

#[cfg(unix)]
fn generate_fixtures(directory: &Path) {
    fs::create_dir_all(directory).unwrap();
    generate_main_fixture(&directory.join("database.wsldb"));
    generate_buffered_fixture(&directory.join("buffered.wsldb"));
}

#[cfg(not(unix))]
fn generate_fixtures(_directory: &Path) {
    panic!("format fixtures must be regenerated on a Unix atomic-replacement path");
}

#[cfg(unix)]
fn generate_main_fixture(path: &Path) {
    remove_fixture_files(path);
    let database = Database::open(path).unwrap();
    let mut first = database.write().unwrap();
    for (key, value) in base_main_entries() {
        first.put(&key, &value).unwrap();
    }
    first.commit().unwrap();

    let mut second = database.write().unwrap();
    for index in [3_u32, 7, 200] {
        second.delete(format!("key-{index:04}").as_bytes()).unwrap();
    }
    second.put(b"key-0042", b"updated").unwrap();
    second.put(b"omega", b"last").unwrap();
    second.commit().unwrap();
    let verified = assert_database(&database, &main_entries());
    assert!(verified.tree_depth >= 2);
    assert!(verified.overflow_pages > 0);
    drop(database);
    remove_if_present(&sidecar(path, ".lock"));
}

#[cfg(unix)]
fn generate_buffered_fixture(path: &Path) {
    remove_fixture_files(path);
    let database = Database::open_with_options(
        path,
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let mut first = database.write().unwrap();
    first.put(b"alpha", b"one").unwrap();
    first.put(b"beta", b"two").unwrap();
    first.put(b"deleted", b"old").unwrap();
    first.commit().unwrap();

    let mut second = database.write().unwrap();
    second.put(b"alpha", b"new").unwrap();
    second.delete(b"deleted").unwrap();
    second.put(b"overflow", &vec![0x5a; 25_000]).unwrap();
    second.commit().unwrap();
    database.flush().unwrap();

    let mut third = database.write().unwrap();
    third.put(b"beta", b"tail").unwrap();
    third.commit().unwrap();
    let mut fourth = database.write().unwrap();
    fourth.put(b"gamma", b"three").unwrap();
    fourth.commit().unwrap();
    assert_database(&database, &buffered_entries());
    drop(database);
    remove_if_present(&sidecar(path, ".lock"));
}

fn base_main_entries() -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut entries = BTreeMap::new();
    for index in 0..300_u32 {
        entries.insert(
            format!("key-{index:04}").into_bytes(),
            format!("value-{index:04}-{}", "x".repeat(80)).into_bytes(),
        );
    }
    entries.insert(b"overflow".to_vec(), vec![0x5a; 25_000]);
    entries
}

fn main_entries() -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut entries = base_main_entries();
    for index in [3_u32, 7, 200] {
        entries.remove(format!("key-{index:04}").as_bytes());
    }
    entries.insert(b"key-0042".to_vec(), b"updated".to_vec());
    entries.insert(b"omega".to_vec(), b"last".to_vec());
    entries
}

fn buffered_entries() -> BTreeMap<Vec<u8>, Vec<u8>> {
    BTreeMap::from([
        (b"alpha".to_vec(), b"new".to_vec()),
        (b"beta".to_vec(), b"tail".to_vec()),
        (b"gamma".to_vec(), b"three".to_vec()),
        (b"overflow".to_vec(), vec![0x5a; 25_000]),
    ])
}

#[cfg(unix)]
fn remove_fixture_files(path: &Path) {
    remove_if_present(path);
    for suffix in [".lock", ".wal"] {
        remove_if_present(&sidecar(path, suffix));
    }
}

#[cfg(unix)]
fn remove_if_present(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => panic!("failed to remove {}: {error}", path.display()),
    }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}
