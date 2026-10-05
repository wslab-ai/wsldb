mod support;

use std::fs;

use support::TestFile;
use wsldb::Database;

#[test]
fn active_compaction_preserves_and_skips_a_stale_replacement_name() {
    let source = TestFile::new("replacement-collision");
    let database = Database::open(source.path()).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"key", b"value").unwrap();
    writer.commit().unwrap();
    drop(database);

    // Initial database publication consumes sequence zero in this isolated
    // integration-test process. Simulate an orphan left by an earlier process
    // at the first active-compaction candidate.
    let file_name = source.path().file_name().unwrap().to_string_lossy();
    let orphan = source.path().with_file_name(format!(
        "{file_name}.wsldb-replacement-{}-1",
        std::process::id()
    ));
    fs::write(&orphan, b"do-not-delete").unwrap();

    Database::compact_file(source.path()).unwrap();

    assert_eq!(fs::read(&orphan).unwrap(), b"do-not-delete");
    let reopened = Database::open(source.path()).unwrap();
    assert_eq!(
        reopened.read().unwrap().get(b"key").unwrap(),
        Some(b"value".to_vec())
    );
    reopened.verify().unwrap();
}
