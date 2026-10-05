mod support;

use std::sync::{Arc, Barrier};
use std::thread;

use support::TestFile;
use wsldb::{Database, DatabaseOptions};

#[test]
fn concurrent_first_reads_initialize_one_index_and_later_commits_update_it() {
    let file = TestFile::new("lazy-point-index");
    let database = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let mut writer = database.write().unwrap();
    for index in 0..100_u32 {
        writer
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    writer.commit().unwrap();

    let barrier = Arc::new(Barrier::new(16));
    let readers = (0..16)
        .map(|index| {
            let database = database.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                assert_eq!(
                    database.read().unwrap().get(b"key-0042").unwrap(),
                    Some(b"value-0042".to_vec()),
                    "reader {index} observed an incomplete index"
                );
            })
        })
        .collect::<Vec<_>>();
    for reader in readers {
        reader.join().unwrap();
    }

    let mut update = database.write().unwrap();
    update.put(b"key-0042", b"updated").unwrap();
    update.put(b"key-0100", b"inserted").unwrap();
    update.commit().unwrap();
    let reader = database.read().unwrap();
    assert_eq!(reader.get(b"key-0042").unwrap(), Some(b"updated".to_vec()));
    assert_eq!(reader.get(b"key-0100").unwrap(), Some(b"inserted".to_vec()));
}
