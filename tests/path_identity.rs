mod support;

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::path::{Path, PathBuf};

#[cfg(unix)]
use support::TestFile;
#[cfg(unix)]
use wsldb::{Database, DatabaseOptions, Error, WriteBatch};

#[cfg(unix)]
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(unix)]
#[test]
fn symbolic_link_alias_uses_the_canonical_lock_and_wal_paths() {
    let source = TestFile::new("canonical-path-source");
    let alias = TestFile::new("canonical-path-alias");
    let database = Database::open(source.path()).unwrap();
    drop(database);
    std::os::unix::fs::symlink(source.path(), alias.path()).unwrap();

    let options = DatabaseOptions::new().write_buffer_bytes(1024 * 1024);
    let aliased = Database::open_with_options(alias.path(), options).unwrap();
    let mut writer = aliased.write().unwrap();
    writer.put(b"canonical", b"paths").unwrap();
    writer.commit().unwrap();

    assert!(sidecar(source.path(), ".lock").exists());
    assert!(sidecar(source.path(), ".wal").exists());
    assert!(!sidecar(alias.path(), ".lock").exists());
    assert!(!sidecar(alias.path(), ".wal").exists());
    assert_eq!(
        Database::open(source.path())
            .unwrap()
            .read()
            .unwrap()
            .get(b"canonical")
            .unwrap(),
        Some(b"paths".to_vec())
    );
}

#[cfg(unix)]
#[test]
fn hard_link_alias_is_rejected_before_it_can_use_an_independent_lock() {
    let source = TestFile::new("hard-link-source");
    let alias = TestFile::new("hard-link-alias");
    let database = Database::open(source.path()).unwrap();
    fs::hard_link(source.path(), alias.path()).unwrap();

    assert!(matches!(
        Database::open(alias.path()),
        Err(Error::InvalidInput(
            "database files with hard links are not supported"
        ))
    ));

    let mut writer = database.write().unwrap();
    writer.put(b"original", b"remains usable").unwrap();
    writer.commit().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"original").unwrap(),
        Some(b"remains usable".to_vec())
    );
}

#[cfg(unix)]
#[test]
fn direct_commit_rejects_a_byte_identical_replacement_inode() {
    let source = TestFile::new("replaced-main-source");
    let replacement = TestFile::new("replaced-main-copy");
    let database = Database::open(source.path()).unwrap();
    let mut initial = database.write().unwrap();
    initial.put(b"stable", b"value").unwrap();
    initial.commit().unwrap();

    fs::copy(source.path(), replacement.path()).unwrap();
    fs::rename(replacement.path(), source.path()).unwrap();

    assert!(matches!(database.flush(), Err(Error::WriteConflict)));
    let mut stale = database.write().unwrap();
    stale.put(b"stale", b"must-not-commit").unwrap();
    assert!(matches!(stale.commit(), Err(Error::WriteConflict)));

    database.refresh().unwrap();
    let mut current = database.write().unwrap();
    current.put(b"current", b"committed").unwrap();
    current.commit().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"current").unwrap(),
        Some(b"committed".to_vec())
    );
    assert_eq!(database.read().unwrap().get(b"stale").unwrap(), None);
}

#[cfg(unix)]
#[test]
fn refresh_rejects_divergent_main_contents_at_the_same_transaction() {
    let source = TestFile::new("divergent-live-main-source");
    let alternate = TestFile::new("divergent-live-main-alternate");
    let saved = TestFile::new("divergent-live-main-saved");
    let replacement = TestFile::new("divergent-live-main-replacement");
    let database = Database::open(source.path()).unwrap();
    let mut initial = database.write().unwrap();
    initial.put(b"stable", b"value").unwrap();
    let transaction = initial.commit().unwrap();

    let alternate_database = Database::open(alternate.path()).unwrap();
    let mut alternate_writer = alternate_database.write().unwrap();
    alternate_writer.put(b"stable", b"different").unwrap();
    assert_eq!(alternate_writer.commit().unwrap(), transaction);
    drop(alternate_database);

    fs::copy(source.path(), saved.path()).unwrap();
    fs::copy(alternate.path(), replacement.path()).unwrap();
    fs::rename(replacement.path(), source.path()).unwrap();

    assert!(matches!(database.refresh(), Err(Error::WriteConflict)));
    assert_eq!(database.stats().unwrap().transaction_id, transaction);
    assert_eq!(
        database.read().unwrap().get(b"stable").unwrap(),
        Some(b"value".to_vec())
    );

    fs::rename(saved.path(), source.path()).unwrap();
    assert_eq!(database.refresh().unwrap(), transaction);
    let mut current = database.write().unwrap();
    current.put(b"current", b"committed").unwrap();
    current.commit().unwrap();
}

#[cfg(unix)]
#[test]
fn buffered_commits_reject_a_byte_identical_main_replacement_before_wal_append() {
    let source = TestFile::new("buffered-replaced-main-source");
    let replacement = TestFile::new("buffered-replaced-main-copy");
    let options = DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024);
    let database = Database::open_with_options(source.path(), options).unwrap();
    let mut initial = database.write().unwrap();
    initial.put(b"stable", b"value").unwrap();
    initial.commit().unwrap();

    let wal = sidecar(source.path(), ".wal");
    let wal_bytes = fs::metadata(&wal).unwrap().len();
    assert!(wal_bytes > 0);
    fs::copy(source.path(), replacement.path()).unwrap();
    fs::rename(replacement.path(), source.path()).unwrap();

    let mut stale = database.write().unwrap();
    stale.put(b"stale", b"must-not-commit").unwrap();
    assert!(matches!(stale.commit(), Err(Error::WriteConflict)));
    let mut explicit = WriteBatch::new();
    explicit.put(b"explicit", b"must-not-commit").unwrap();
    assert!(matches!(
        database.commit_group(vec![explicit]),
        Err(Error::WriteConflict)
    ));
    let mut automatic = WriteBatch::new();
    automatic.put(b"automatic", b"must-not-commit").unwrap();
    assert!(matches!(
        database.commit_batch(automatic),
        Err(Error::WriteConflict)
    ));
    assert_eq!(fs::metadata(&wal).unwrap().len(), wal_bytes);

    database.refresh().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"stable").unwrap(),
        Some(b"value".to_vec())
    );
    let mut current = database.write().unwrap();
    current.put(b"current", b"committed").unwrap();
    current.commit().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"current").unwrap(),
        Some(b"committed".to_vec())
    );
    assert_eq!(database.read().unwrap().get(b"stale").unwrap(), None);
    assert_eq!(database.read().unwrap().get(b"explicit").unwrap(), None);
    assert_eq!(database.read().unwrap().get(b"automatic").unwrap(), None);
}

#[cfg(unix)]
#[test]
fn buffered_commits_reject_a_byte_identical_wal_replacement_before_append() {
    let source = TestFile::new("buffered-replaced-wal-source");
    let replacement = TestFile::new("buffered-replaced-wal-copy");
    let backup = TestFile::new("buffered-replaced-wal-backup");
    let compacted = TestFile::new("buffered-replaced-wal-compacted");
    let options = DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024);
    let database = Database::open_with_options(source.path(), options).unwrap();
    let mut initial = database.write().unwrap();
    initial.put(b"stable", b"value").unwrap();
    initial.commit().unwrap();

    let wal = sidecar(source.path(), ".wal");
    let wal_bytes = fs::metadata(&wal).unwrap().len();
    assert!(wal_bytes > 0);
    fs::copy(&wal, replacement.path()).unwrap();
    fs::rename(replacement.path(), &wal).unwrap();

    let mut stale = database.write().unwrap();
    stale.put(b"stale", b"must-not-commit").unwrap();
    assert!(matches!(stale.commit(), Err(Error::WriteConflict)));
    let mut explicit = WriteBatch::new();
    explicit.put(b"explicit", b"must-not-commit").unwrap();
    assert!(matches!(
        database.commit_group(vec![explicit]),
        Err(Error::WriteConflict)
    ));
    let mut automatic = WriteBatch::new();
    automatic.put(b"automatic", b"must-not-commit").unwrap();
    assert!(matches!(
        database.commit_batch(automatic),
        Err(Error::WriteConflict)
    ));
    assert!(matches!(
        database.operational_stats(),
        Err(Error::WriteConflict)
    ));
    assert!(matches!(
        database.backup(backup.path()),
        Err(Error::WriteConflict)
    ));
    assert!(matches!(
        database.compact_to(compacted.path()),
        Err(Error::WriteConflict)
    ));
    assert!(matches!(database.compact(), Err(Error::WriteConflict)));
    assert!(matches!(database.flush(), Err(Error::WriteConflict)));
    assert!(!backup.path().exists());
    assert!(!compacted.path().exists());
    assert_eq!(fs::metadata(&wal).unwrap().len(), wal_bytes);

    database.refresh().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"stable").unwrap(),
        Some(b"value".to_vec())
    );
    let mut current = database.write().unwrap();
    current.put(b"current", b"committed").unwrap();
    current.commit().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"current").unwrap(),
        Some(b"committed".to_vec())
    );
    assert_eq!(database.read().unwrap().get(b"stale").unwrap(), None);
    assert_eq!(database.read().unwrap().get(b"explicit").unwrap(), None);
    assert_eq!(database.read().unwrap().get(b"automatic").unwrap(), None);
}

#[cfg(unix)]
#[test]
fn refresh_rejects_a_missing_nonempty_wal_without_recreating_it() {
    let source = TestFile::new("missing-live-wal-source");
    let moved = TestFile::new("missing-live-wal-moved");
    let options = DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024);
    let database = Database::open_with_options(source.path(), options).unwrap();
    let mut initial = database.write().unwrap();
    initial.put(b"stable", b"value").unwrap();
    let transaction = initial.commit().unwrap();

    let wal = sidecar(source.path(), ".wal");
    fs::rename(&wal, moved.path()).unwrap();
    assert!(matches!(database.refresh(), Err(Error::WriteConflict)));
    assert!(!wal.exists());
    assert_eq!(
        database.read().unwrap().get(b"stable").unwrap(),
        Some(b"value".to_vec())
    );
    let mut stale = database.write().unwrap();
    stale.put(b"stale", b"must-not-commit").unwrap();
    assert!(matches!(stale.commit(), Err(Error::WriteConflict)));

    fs::rename(moved.path(), &wal).unwrap();
    assert_eq!(database.refresh().unwrap(), transaction);
    let mut current = database.write().unwrap();
    current.put(b"current", b"committed").unwrap();
    current.commit().unwrap();
    assert_eq!(database.read().unwrap().get(b"stale").unwrap(), None);
    assert_eq!(
        database.read().unwrap().get(b"current").unwrap(),
        Some(b"committed".to_vec())
    );
}

#[cfg(unix)]
#[test]
fn refresh_rejects_divergent_wal_contents_at_the_same_transaction() {
    let source = TestFile::new("divergent-live-wal-source");
    let alternate = TestFile::new("divergent-live-wal-alternate");
    let saved = TestFile::new("divergent-live-wal-saved");
    let replacement = TestFile::new("divergent-live-wal-replacement");
    let options = DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024);
    let database = Database::open_with_options(source.path(), options.clone()).unwrap();
    let mut initial = database.write().unwrap();
    initial.put(b"stable", b"value").unwrap();
    let transaction = initial.commit().unwrap();

    let alternate_database = Database::open_with_options(alternate.path(), options).unwrap();
    let mut alternate_writer = alternate_database.write().unwrap();
    alternate_writer.put(b"stable", b"different").unwrap();
    assert_eq!(alternate_writer.commit().unwrap(), transaction);
    drop(alternate_database);

    let wal = sidecar(source.path(), ".wal");
    fs::copy(&wal, saved.path()).unwrap();
    fs::copy(sidecar(alternate.path(), ".wal"), replacement.path()).unwrap();
    fs::rename(replacement.path(), &wal).unwrap();

    assert!(matches!(database.refresh(), Err(Error::WriteConflict)));
    assert_eq!(database.stats().unwrap().transaction_id, transaction);
    assert_eq!(
        database.read().unwrap().get(b"stable").unwrap(),
        Some(b"value".to_vec())
    );

    fs::rename(saved.path(), &wal).unwrap();
    assert_eq!(database.refresh().unwrap(), transaction);
    let mut current = database.write().unwrap();
    current.put(b"current", b"committed").unwrap();
    current.commit().unwrap();
}

#[cfg(unix)]
#[test]
fn first_buffered_commit_rejects_a_replacement_before_wal_append() {
    let source = TestFile::new("empty-wal-replaced-main-source");
    let replacement = TestFile::new("empty-wal-replaced-main-copy");
    let options = DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024);
    let database = Database::open_with_options(source.path(), options).unwrap();
    let wal = sidecar(source.path(), ".wal");
    assert_eq!(fs::metadata(&wal).unwrap().len(), 0);

    fs::copy(source.path(), replacement.path()).unwrap();
    fs::rename(replacement.path(), source.path()).unwrap();
    let mut stale = database.write().unwrap();
    stale.put(b"stale", b"must-not-commit").unwrap();
    assert!(matches!(stale.commit(), Err(Error::WriteConflict)));
    assert_eq!(fs::metadata(&wal).unwrap().len(), 0);

    database.refresh().unwrap();
    let mut current = database.write().unwrap();
    current.put(b"current", b"committed").unwrap();
    current.commit().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"current").unwrap(),
        Some(b"committed".to_vec())
    );
    assert_eq!(database.read().unwrap().get(b"stale").unwrap(), None);
}

#[cfg(unix)]
#[test]
fn buffered_main_dependent_operations_reject_a_replacement_inode() {
    let source = TestFile::new("buffered-operation-main-source");
    let replacement = TestFile::new("buffered-operation-main-copy");
    let backup = TestFile::new("buffered-operation-backup");
    let compacted = TestFile::new("buffered-operation-compacted");
    let options = DatabaseOptions::new().write_buffer_bytes(64 * 1024 * 1024);
    let database = Database::open_with_options(source.path(), options).unwrap();
    let mut initial = database.write().unwrap();
    initial.put(b"stable", b"value").unwrap();
    initial.commit().unwrap();

    let wal = sidecar(source.path(), ".wal");
    let wal_bytes = fs::metadata(&wal).unwrap().len();
    fs::copy(source.path(), replacement.path()).unwrap();
    fs::rename(replacement.path(), source.path()).unwrap();

    assert!(matches!(
        database.operational_stats(),
        Err(Error::WriteConflict)
    ));
    assert!(matches!(
        database.backup(backup.path()),
        Err(Error::WriteConflict)
    ));
    assert!(matches!(
        database.compact_to(compacted.path()),
        Err(Error::WriteConflict)
    ));
    assert!(matches!(database.compact(), Err(Error::WriteConflict)));
    assert!(matches!(database.flush(), Err(Error::WriteConflict)));
    assert!(!backup.path().exists());
    assert!(!compacted.path().exists());
    assert_eq!(fs::metadata(&wal).unwrap().len(), wal_bytes);

    database.refresh().unwrap();
    database.verify().unwrap();
    assert_eq!(
        database.read().unwrap().get(b"stable").unwrap(),
        Some(b"value".to_vec())
    );
}

#[cfg(unix)]
#[test]
fn published_temporary_hard_link_is_cleaned_before_identity_validation() {
    let source = TestFile::new("published-temp-source");
    let database = Database::open(source.path()).unwrap();
    drop(database);
    let file_name = source.path().file_name().unwrap().to_string_lossy();
    let temporary = source
        .path()
        .with_file_name(format!("{file_name}.wsldb-tmp-crashed-creation"));
    fs::hard_link(source.path(), &temporary).unwrap();

    let reopened = Database::open(source.path()).unwrap();

    assert!(!temporary.exists());
    reopened.verify().unwrap();
}

#[cfg(unix)]
#[test]
fn removing_the_lock_sidecar_invalidates_existing_handles_before_commit() {
    let source = TestFile::new("removed-lock-sidecar");
    let database = Database::open(source.path()).unwrap();
    let mut writer = database.write().unwrap();
    writer.put(b"stale-lock", b"must-not-commit").unwrap();
    fs::remove_file(sidecar(source.path(), ".lock")).unwrap();

    assert!(matches!(
        writer.commit(),
        Err(Error::InvalidInput(
            "database lock file was removed; reopen the database"
        ))
    ));

    let reopened = Database::open(source.path()).unwrap();
    assert_eq!(reopened.read().unwrap().get(b"stale-lock").unwrap(), None);
    let mut current = reopened.write().unwrap();
    current.put(b"current-lock", b"committed").unwrap();
    current.commit().unwrap();
    reopened.verify().unwrap();
}

#[cfg(unix)]
#[test]
fn renaming_or_replacing_the_lock_sidecar_invalidates_existing_handles() {
    let source = TestFile::new("renamed-lock-sidecar");
    let database = Database::open(source.path()).unwrap();
    let lock = sidecar(source.path(), ".lock");
    let moved = sidecar(source.path(), ".lock.moved");
    fs::rename(&lock, &moved).unwrap();

    let mut missing = database.write().unwrap();
    missing.put(b"missing-lock", b"must-not-commit").unwrap();
    assert!(matches!(
        missing.commit(),
        Err(Error::InvalidInput(
            "database lock sidecar identity changed; reopen the database"
        ))
    ));

    let reopened = Database::open(source.path()).unwrap();
    let mut stale = database.write().unwrap();
    stale.put(b"replacement-lock", b"must-not-commit").unwrap();
    assert!(matches!(
        stale.commit(),
        Err(Error::InvalidInput(
            "database lock sidecar identity changed; reopen the database"
        ))
    ));

    let mut current = reopened.write().unwrap();
    current.put(b"current-lock", b"committed").unwrap();
    current.commit().unwrap();
    assert_eq!(reopened.verify().unwrap().entries, 1);
    drop(database);
    fs::remove_file(moved).unwrap();
}

#[cfg(unix)]
#[test]
fn non_regular_database_and_lock_paths_are_rejected_before_creation() {
    let database_path = TestFile::new("database-directory");
    fs::create_dir(database_path.path()).unwrap();
    assert!(matches!(
        Database::open(database_path.path()),
        Err(Error::InvalidInput(
            "database path must refer to a regular file"
        ))
    ));
    fs::remove_dir(database_path.path()).unwrap();

    let source = TestFile::new("symbolic-lock-source");
    let redirected = TestFile::new("symbolic-lock-target");
    fs::write(redirected.path(), b"must-not-be-used-as-a-lock").unwrap();
    std::os::unix::fs::symlink(redirected.path(), sidecar(source.path(), ".lock")).unwrap();
    assert!(matches!(
        Database::open(source.path()),
        Err(Error::InvalidInput(
            "database lock sidecar must be a regular file, not a symbolic link"
        ))
    ));
    assert!(!source.path().exists());
    assert_eq!(
        fs::read(redirected.path()).unwrap(),
        b"must-not-be-used-as-a-lock"
    );
    fs::remove_file(sidecar(source.path(), ".lock")).unwrap();
    fs::hard_link(redirected.path(), sidecar(source.path(), ".lock")).unwrap();
    assert!(matches!(
        Database::open(source.path()),
        Err(Error::InvalidInput(
            "database lock sidecar must not have hard links"
        ))
    ));
    assert!(!source.path().exists());
}

#[cfg(unix)]
#[test]
fn symbolic_and_hard_linked_wal_sidecars_are_rejected() {
    let source = TestFile::new("symbolic-wal-source");
    drop(Database::open(source.path()).unwrap());
    let redirected = TestFile::new("symbolic-wal-target");
    fs::write(redirected.path(), b"must-not-be-used-as-a-wal").unwrap();
    std::os::unix::fs::symlink(redirected.path(), sidecar(source.path(), ".wal")).unwrap();
    assert!(matches!(
        Database::open(source.path()),
        Err(Error::InvalidInput(
            "write-ahead sidecar must be a regular file, not a symbolic link"
        ))
    ));
    assert_eq!(
        fs::read(redirected.path()).unwrap(),
        b"must-not-be-used-as-a-wal"
    );
    fs::remove_file(sidecar(source.path(), ".wal")).unwrap();

    let buffered = Database::open_with_options(
        source.path(),
        DatabaseOptions::new().write_buffer_bytes(1024 * 1024),
    )
    .unwrap();
    let mut writer = buffered.write().unwrap();
    writer.put(b"buffered", b"durable").unwrap();
    writer.commit().unwrap();
    drop(buffered);
    let alias = TestFile::new("hard-linked-wal");
    fs::hard_link(sidecar(source.path(), ".wal"), alias.path()).unwrap();
    assert!(matches!(
        Database::open(source.path()),
        Err(Error::InvalidInput(
            "write-ahead sidecar must not have hard links"
        ))
    ));
}
