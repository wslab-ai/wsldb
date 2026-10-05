mod support;

use support::TestFile;
use wsldb::{Database, DatabaseOptions};

#[test]
fn buffered_verification_matches_a_materialized_mixed_snapshot() {
    let source = TestFile::new("buffered-streaming-verification-source");
    let compacted = TestFile::new("buffered-streaming-verification-compact");
    let database = Database::open_with_options(
        source.path(),
        DatabaseOptions::new().write_buffer_bytes(4 * 1024),
    )
    .unwrap();

    let mut initial = database.write().unwrap();
    for index in 0..200_u32 {
        initial
            .put(
                format!("key-{index:04}").as_bytes(),
                format!("initial-value-{index:04}").as_bytes(),
            )
            .unwrap();
    }
    initial.commit().unwrap();

    let mut tail = database.write().unwrap();
    tail.delete(b"key-0001").unwrap();
    tail.put(b"key-0042", b"updated").unwrap();
    tail.put(b"key-9999", b"inserted").unwrap();
    tail.commit().unwrap();

    let buffered = database.verify().unwrap();
    let report = database.compact_to(compacted.path()).unwrap();
    let materialized = Database::open(compacted.path()).unwrap().verify().unwrap();
    assert_eq!(buffered.entries, 200);
    assert_eq!(
        buffered.logical_hash,
        report.compacted_snapshot.logical_hash
    );
    assert_eq!(buffered.logical_hash, materialized.logical_hash);
}
