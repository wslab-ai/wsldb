mod support;

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};

use support::TestFile;
use wsldb::{Database, Error};

const PAGE_SIZE: usize = 4096;

#[test]
fn a_single_head_in_the_wrong_parity_slot_is_rejected() {
    let file = TestFile::new("wrong-metadata-slot");
    let database = Database::open(file.path()).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"value").unwrap();
    writer.commit().unwrap();
    drop(database);

    let mut storage = OpenOptions::new()
        .read(true)
        .write(true)
        .open(file.path())
        .unwrap();
    storage.seek(SeekFrom::Start(PAGE_SIZE as u64)).unwrap();
    let mut odd_head = [0_u8; PAGE_SIZE];
    storage.read_exact(&mut odd_head).unwrap();
    storage.seek(SeekFrom::Start(0)).unwrap();
    storage.write_all(&odd_head).unwrap();
    storage.seek(SeekFrom::Start(PAGE_SIZE as u64)).unwrap();
    storage.write_all(&[0_u8; PAGE_SIZE]).unwrap();
    storage.sync_all().unwrap();
    drop(storage);

    assert!(matches!(
        Database::open(file.path()),
        Err(Error::Corrupt(_))
    ));
}
