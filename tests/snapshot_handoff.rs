#![cfg(unix)]

mod support;

use support::TestFile;
use wsldb::{Database, DatabaseOptions};

#[test]
fn shared_point_index_does_not_leak_newer_versions_across_handoff() {
    let file = TestFile::new("buffer-snapshot-shared-index");
    let database = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let mut first = database.write().unwrap();
    first.put(b"alpha", b"one").unwrap();
    first.put(b"beta", b"two").unwrap();
    first.commit().unwrap();
    let old_reader = database.read().unwrap();
    assert_eq!(old_reader.get(b"alpha").unwrap(), Some(b"one".to_vec()));

    let mut second = database.write().unwrap();
    second.put(b"alpha", b"updated").unwrap();
    second.delete(b"beta").unwrap();
    second.put(b"gamma", b"three").unwrap();
    second.commit().unwrap();
    let before_flush = database.read().unwrap();
    assert_eq!(
        before_flush.get(b"alpha").unwrap(),
        Some(b"updated".to_vec())
    );
    assert_eq!(before_flush.get(b"beta").unwrap(), None);

    database.flush().unwrap();
    let mut third = database.write().unwrap();
    third.put(b"alpha", b"after-flush").unwrap();
    third.put(b"delta", b"four").unwrap();
    third.commit().unwrap();

    assert_eq!(old_reader.get(b"alpha").unwrap(), Some(b"one".to_vec()));
    assert_eq!(old_reader.get(b"beta").unwrap(), Some(b"two".to_vec()));
    assert_eq!(old_reader.get(b"delta").unwrap(), None);
    assert_eq!(
        before_flush.get(b"alpha").unwrap(),
        Some(b"updated".to_vec())
    );
    assert_eq!(before_flush.get(b"beta").unwrap(), None);
    assert_eq!(before_flush.get(b"gamma").unwrap(), Some(b"three".to_vec()));
    assert_eq!(before_flush.get(b"delta").unwrap(), None);
    let newest = database.read().unwrap();
    assert_eq!(newest.get(b"alpha").unwrap(), Some(b"after-flush".to_vec()));
    assert_eq!(newest.get(b"delta").unwrap(), Some(b"four".to_vec()));
    drop(newest);
    drop(before_flush);
    drop(old_reader);
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 3);
    assert_eq!(
        reopened.read().unwrap().get(b"alpha").unwrap(),
        Some(b"after-flush".to_vec())
    );
    assert_eq!(
        reopened.read().unwrap().get(b"delta").unwrap(),
        Some(b"four".to_vec())
    );
}
