mod support;

use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error, WriteBatch};

#[test]
fn recovery_without_buffer_options_returns_to_direct_mode_after_flush() {
    let file = TestFile::new("buffered-recovery-mode");
    let buffered = Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024),
    )
    .unwrap();
    let mut initial = buffered.write().unwrap();
    initial.put(b"alpha", b"one").unwrap();
    initial.commit().unwrap();
    drop(buffered);

    let recovered = Database::open(file.path()).unwrap();
    assert_eq!(
        recovered.read().unwrap().get(b"alpha").unwrap(),
        Some(b"one".to_vec())
    );
    assert!(matches!(
        recovered.commit_group(vec![WriteBatch::new()]),
        Err(Error::InvalidInput(
            "group commit requires durable write buffering"
        ))
    ));

    recovered.flush().unwrap();
    let before_direct = recovered.operational_stats().unwrap().writes;
    let mut direct = recovered.write().unwrap();
    direct.put(b"beta", b"two").unwrap();
    direct.commit().unwrap();
    let after_direct = recovered.operational_stats().unwrap().writes;
    assert!(after_direct.data_page_bytes > before_direct.data_page_bytes);
    assert_eq!(
        after_direct.write_buffer_bytes,
        before_direct.write_buffer_bytes
    );
    drop(recovered);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap().transaction_id.value(), 2);
    assert_eq!(
        reopened.read().unwrap().get(b"beta").unwrap(),
        Some(b"two".to_vec())
    );
}
