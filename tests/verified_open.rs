mod support;

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error};

const PAGE_SIZE: u64 = 4096;
const BUFFER_BYTES: u64 = 64 * 1024 * 1024;

#[test]
fn verified_open_falls_back_from_deep_corruption_without_a_wal() {
    let file = TestFile::new("verified-deep-fallback");
    let backup = TestFile::new("verified-deep-fallback-backup");
    let compacted = TestFile::new("verified-deep-fallback-compacted");
    let newest_root = create_two_generations(file.path());
    flip_byte(file.path(), (newest_root - 1) * PAGE_SIZE);

    let fast = Database::open(file.path()).unwrap();
    assert_eq!(fast.stats().unwrap().transaction_id.value(), 2);
    assert!(matches!(
        fast.read().unwrap().get(b"key"),
        Err(Error::Corrupt(_))
    ));
    drop(fast);

    let recovered =
        Database::open_with_options(file.path(), DatabaseOptions::new().verify_on_open(true))
            .unwrap();
    let reader = recovered.read().unwrap();
    assert_eq!(reader.transaction_id().value(), 1);
    assert_eq!(reader.get(b"key").unwrap(), Some(b"one".to_vec()));
    assert_eq!(recovered.verify().unwrap().entries, 1);
    assert!(matches!(
        recovered.operational_stats(),
        Err(Error::Corrupt(_))
    ));

    let backup_report = recovered.backup(backup.path()).unwrap();
    assert_eq!(backup_report.snapshot.transaction_id.value(), 1);
    assert_recovered_export(backup.path());

    let compact_report = recovered.compact_to(compacted.path()).unwrap();
    assert_eq!(compact_report.source_transaction_id.value(), 1);
    assert_eq!(compact_report.compacted_snapshot.transaction_id.value(), 1);
    assert_recovered_export(compacted.path());
}

#[test]
fn buffered_verified_recovery_view_rejects_writes_and_exports() {
    let file = TestFile::new("verified-buffered-recovery");
    let compacted = TestFile::new("verified-buffered-recovery-compacted");
    let newest_root = create_two_generations(file.path());
    flip_byte(file.path(), (newest_root - 1) * PAGE_SIZE);

    let recovered = Database::open_with_options(
        file.path(),
        DatabaseOptions::new()
            .write_buffer_bytes(BUFFER_BYTES)
            .verify_on_open(true),
    )
    .unwrap();
    let mut writer = recovered.write().unwrap();
    writer.put(b"new", b"must-not-commit").unwrap();
    assert!(matches!(writer.commit(), Err(Error::WriteConflict)));
    assert_eq!(
        std::fs::metadata(sidecar(file.path(), ".wal"))
            .unwrap()
            .len(),
        0
    );

    let report = recovered.compact_to(compacted.path()).unwrap();
    assert_eq!(report.source_transaction_id.value(), 1);
    assert_recovered_export(compacted.path());
}

#[test]
fn verified_open_does_not_fall_back_across_a_nonempty_wal() {
    let file = TestFile::new("verified-wal-no-fallback");
    let newest_root = create_two_generations(file.path());
    let buffered = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(BUFFER_BYTES),
    )
    .unwrap();
    let mut writer = buffered.write().unwrap();
    writer.put(b"wal-key", b"durable").unwrap();
    writer.commit().unwrap();
    drop(buffered);

    flip_byte(file.path(), (newest_root - 1) * PAGE_SIZE);

    assert!(matches!(
        Database::open_with_options(
            file.path(),
            DatabaseOptions::new()
                .write_buffer_bytes(BUFFER_BYTES)
                .verify_on_open(true)
        ),
        Err(Error::Corrupt(_))
    ));
}

fn create_two_generations(path: &Path) -> u64 {
    let database = Database::open(path).unwrap();
    let mut first = database.write().unwrap();
    first.put(b"key", b"one").unwrap();
    first.commit().unwrap();
    let mut second = database.write().unwrap();
    second.put(b"key", &vec![b'x'; 25_000]).unwrap();
    second.commit().unwrap();
    drop(database);

    let newest_root = read_u64_at(path, 24);
    assert!(newest_root > 3);
    newest_root
}

fn assert_recovered_export(path: &Path) {
    let exported =
        Database::open_with_options(path, DatabaseOptions::new().verify_on_open(true)).unwrap();
    let reader = exported.read().unwrap();
    assert_eq!(reader.transaction_id().value(), 1);
    assert_eq!(reader.get(b"key").unwrap(), Some(b"one".to_vec()));
    assert_eq!(exported.verify().unwrap().entries, 1);
}

fn sidecar(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
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
    byte[0] ^= 0xff;
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&byte).unwrap();
    file.sync_all().unwrap();
}
