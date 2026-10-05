mod support;

#[cfg(any(feature = "crash-injection", feature = "fault-injection"))]
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::Duration;

use support::TestFile;
use wsldb::{Database, DatabaseOptions, Error, WriteBatch};

const BUFFER_BYTES: u64 = 64 * 1024 * 1024;

#[test]
fn ordered_batches_share_one_durable_sync_and_recover() {
    let file = TestFile::new("group-commit");
    let database = buffered(&file, BUFFER_BYTES);
    let old_reader = database.read().unwrap();

    let mut first = WriteBatch::new();
    first.put(b"ephemeral", b"one").unwrap();
    first.put(b"shared", b"first").unwrap();
    let mut second = WriteBatch::new();
    second.delete(b"ephemeral").unwrap();
    second.put(b"shared", b"second").unwrap();
    second.put(b"stable", b"value").unwrap();

    let transaction = database.commit_group(vec![first, second]).unwrap();
    assert_eq!(transaction.value(), 2);
    assert!(old_reader.is_empty());
    assert_eq!(database.stats().unwrap().entries, 2);
    assert_eq!(database.read().unwrap().get(b"ephemeral").unwrap(), None);
    assert_eq!(
        database.read().unwrap().get(b"shared").unwrap(),
        Some(b"second".to_vec())
    );
    assert_eq!(
        database.read().unwrap().get(b"stable").unwrap(),
        Some(b"value".to_vec())
    );

    let stats = database.operational_stats().unwrap();
    assert_eq!(stats.commits.operations, 1);
    assert_eq!(stats.syncs.operations, 1);
    assert_eq!(stats.wal_preflights.operations, 1);
    assert_eq!(stats.wal_encodes.operations, 1);
    assert_eq!(stats.wal_writes.operations, 1);
    assert_eq!(stats.wal_syncs.operations, 1);
    assert!(stats.wal_syncs.p50_nanos >= stats.wal_syncs.max_nanos);
    assert_eq!(stats.wal_syncs.p50_nanos, stats.wal_syncs.p999_nanos);
    assert_eq!(stats.wal_publications.operations, 1);
    assert!(stats.writes.write_buffer_bytes > 0);
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.stats().unwrap().transaction_id, transaction);
    assert_eq!(reopened.verify().unwrap().entries, 2);
    assert_eq!(
        reopened.read().unwrap().get(b"shared").unwrap(),
        Some(b"second".to_vec())
    );
}

#[test]
fn group_commit_can_checkpoint_after_the_shared_sync() {
    let file = TestFile::new("group-checkpoint");
    let database = buffered(&file, 1);
    let mut first = WriteBatch::new();
    first.put(b"first", b"one").unwrap();
    let mut second = WriteBatch::new();
    second.put(b"second", b"two").unwrap();

    let transaction = database.commit_group(vec![first, second]).unwrap();
    assert_eq!(transaction.value(), 2);
    assert_eq!(database.verify().unwrap().entries, 2);
    assert_eq!(database.operational_stats().unwrap().checkpoint_failures, 0);
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.stats().unwrap().transaction_id, transaction);
    assert_eq!(reopened.verify().unwrap().entries, 2);
}

#[test]
fn empty_batches_advance_the_ordered_transaction_chain() {
    let file = TestFile::new("group-empty-batches");
    let database = buffered(&file, BUFFER_BYTES);

    let transaction = database
        .commit_group(vec![WriteBatch::new(), WriteBatch::new()])
        .unwrap();

    assert_eq!(transaction.value(), 2);
    assert!(database.read().unwrap().is_empty());
    drop(database);
    assert_eq!(
        Database::open(file.path())
            .unwrap()
            .stats()
            .unwrap()
            .transaction_id,
        transaction
    );
}

#[test]
fn concurrent_batches_share_syncs_and_receive_their_own_transaction_ids() {
    const THREADS: usize = 16;

    let file = TestFile::new("automatic-group");
    let database = buffered(&file, BUFFER_BYTES);
    let handles: Vec<_> = (0..THREADS).map(|_| database.clone()).collect();
    let ordinary_writer = database.write().unwrap();
    let start = Arc::new(Barrier::new(THREADS + 1));
    let (ready_sender, ready_receiver) = mpsc::channel();
    let workers: Vec<_> = handles
        .into_iter()
        .enumerate()
        .map(|(index, database)| {
            let start = Arc::clone(&start);
            let ready = ready_sender.clone();
            thread::spawn(move || {
                start.wait();
                ready.send(()).unwrap();
                let mut batch = WriteBatch::new();
                batch.put(&u64::try_from(index).unwrap().to_be_bytes(), b"value")?;
                database.commit_batch(batch)
            })
        })
        .collect();
    drop(ready_sender);
    start.wait();
    for _ in 0..THREADS {
        ready_receiver.recv().unwrap();
    }
    thread::sleep(Duration::from_millis(20));
    ordinary_writer.abort();

    let mut transaction_ids: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap().value())
        .collect();
    transaction_ids.sort_unstable();
    assert_eq!(
        transaction_ids,
        (1..=u64::try_from(THREADS).unwrap()).collect::<Vec<_>>()
    );
    let report = database.verify().unwrap();
    assert_eq!(report.entries, THREADS as u64);
    assert_eq!(report.transaction_id.value(), THREADS as u64);
    let operations = database.operational_stats().unwrap();
    assert!(operations.wal_syncs.operations < THREADS as u64);
    assert_eq!(
        operations.wal_preflights.operations,
        operations.wal_syncs.operations
    );
    assert_eq!(
        operations.wal_encodes.operations,
        operations.wal_syncs.operations
    );
    assert_eq!(
        operations.wal_writes.operations,
        operations.wal_syncs.operations
    );
    assert_eq!(
        operations.wal_publications.operations,
        operations.wal_syncs.operations
    );
    drop(database);

    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.verify().unwrap(), report);
}

#[test]
fn automatic_group_commit_advances_empty_batches_and_requires_a_buffer() {
    let file = TestFile::new("automatic-empty");
    let database = buffered(&file, BUFFER_BYTES);
    assert_eq!(database.commit_batch(WriteBatch::new()).unwrap().value(), 1);

    let direct_file = TestFile::new("automatic-direct");
    let direct = Database::open(direct_file.path()).unwrap();
    assert!(matches!(
        direct.commit_batch(WriteBatch::new()),
        Err(Error::InvalidInput(_))
    ));
}

#[cfg(feature = "fault-injection")]
#[test]
fn automatic_group_failures_reach_every_waiter_and_recover() {
    for (label, stage, fail_after_bytes, ambiguous) in [
        ("automatic-partial-write", "positioned_write", "1", false),
        ("automatic-sync", "write_buffer_sync", "0", true),
    ] {
        let file = TestFile::new(label);
        drop(buffered(&file, BUFFER_BYTES));
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("automatic_group_failure_worker")
            .arg("--nocapture")
            .env("WSLDB_AUTO_GROUP_FAILURE_PATH", file.path())
            .env("WSLDB_AUTO_GROUP_FAILURE_AMBIGUOUS", ambiguous.to_string())
            .env("WSLDB_IO_FAILPOINT", stage)
            .env("WSLDB_IO_FAIL_AFTER_BYTES", fail_after_bytes)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "automatic group failure worker failed");

        let recovered = Database::open(file.path()).unwrap();
        let report = recovered.verify().unwrap();
        if ambiguous {
            assert_eq!(report.transaction_id.value(), 5);
            assert_eq!(report.entries, 4);
        } else {
            assert_eq!(report.transaction_id.value(), 0);
            assert_eq!(report.entries, 0);
        }
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn automatic_group_failure_worker() {
    const THREADS: usize = 4;

    let Some(path) = std::env::var_os("WSLDB_AUTO_GROUP_FAILURE_PATH") else {
        return;
    };
    let ambiguous = std::env::var("WSLDB_AUTO_GROUP_FAILURE_AMBIGUOUS").unwrap() == "true";
    let database = Database::open_with_options(
        path,
        DatabaseOptions::new().write_buffer_bytes(BUFFER_BYTES),
    )
    .unwrap();
    let handles: Vec<_> = (0..THREADS).map(|_| database.clone()).collect();
    let ordinary_writer = database.write().unwrap();
    let start = Arc::new(Barrier::new(THREADS + 1));
    let (ready_sender, ready_receiver) = mpsc::channel();
    let workers: Vec<_> = handles
        .into_iter()
        .enumerate()
        .map(|(index, database)| {
            let start = Arc::clone(&start);
            let ready = ready_sender.clone();
            thread::spawn(move || {
                start.wait();
                ready.send(()).unwrap();
                let mut batch = WriteBatch::new();
                batch
                    .put(&u64::try_from(index).unwrap().to_be_bytes(), b"value")
                    .unwrap();
                database.commit_batch(batch)
            })
        })
        .collect();
    drop(ready_sender);
    start.wait();
    for _ in 0..THREADS {
        ready_receiver.recv().unwrap();
    }
    thread::sleep(Duration::from_millis(20));
    ordinary_writer.abort();

    for worker in workers {
        let result = worker.join().unwrap();
        if ambiguous {
            assert!(matches!(result, Err(Error::CommitOutcomeUnknown(_))));
        } else {
            assert!(matches!(result, Err(Error::Io(_))));
        }
    }
    assert_eq!(database.stats().unwrap().transaction_id.value(), 0);
    if ambiguous {
        assert!(matches!(
            database.commit_batch(WriteBatch::new()),
            Err(Error::LockPoisoned("automatic group commit"))
        ));
        assert_eq!(database.refresh().unwrap().value(), 4);
        assert_eq!(database.commit_batch(WriteBatch::new()).unwrap().value(), 5);
    }
}

#[test]
fn group_commit_rejects_invalid_configuration_and_stale_heads() {
    let direct_file = TestFile::new("group-direct");
    let direct = Database::open(direct_file.path()).unwrap();
    assert!(matches!(
        direct.commit_group(Vec::new()),
        Err(Error::InvalidInput(_))
    ));
    let mut batch = WriteBatch::new();
    batch.put(b"key", b"value").unwrap();
    assert!(matches!(
        direct.commit_group(vec![batch]),
        Err(Error::InvalidInput(_))
    ));

    let file = TestFile::new("group-stale");
    let first = buffered(&file, BUFFER_BYTES);
    let stale = buffered(&file, BUFFER_BYTES);
    let mut current_batch = WriteBatch::new();
    current_batch.put(b"current", b"one").unwrap();
    first.commit_group(vec![current_batch]).unwrap();

    let mut stale_batch = WriteBatch::new();
    stale_batch.put(b"stale", b"must-not-commit").unwrap();
    assert!(matches!(
        stale.commit_group(vec![stale_batch]),
        Err(Error::WriteConflict)
    ));
    assert_eq!(stale.refresh().unwrap().value(), 1);
}

#[test]
fn write_batch_validates_and_replaces_mutations() {
    let mut batch = WriteBatch::new();
    assert!(batch.is_empty());
    assert!(matches!(
        batch.put(b"", b"value"),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(batch.delete(b""), Err(Error::InvalidInput(_))));
    batch.put(b"key", b"old").unwrap();
    batch.put(b"key", b"new").unwrap();
    assert_eq!(batch.len(), 1);
}

#[cfg(feature = "crash-injection")]
#[test]
fn crash_after_group_wal_write_recovers_no_or_all_transactions() {
    let file = TestFile::new("group-write-crash");
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("group_commit_crash_worker")
        .arg("--nocapture")
        .env("WSLDB_GROUP_CRASH_PATH", file.path())
        .env("WSLDB_FAILPOINT", "write_buffer_write")
        .env("WSLDB_FAIL_AFTER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "group write failpoint was not reached");

    let recovered = Database::open(file.path()).unwrap();
    let transaction = recovered.stats().unwrap().transaction_id.value();
    let entries = recovered.verify().unwrap().entries;
    assert!(
        (transaction == 0 && entries == 0) || (transaction == 2 && entries == 2),
        "recovery published a partial group: transaction={transaction}, entries={entries}"
    );
    assert_eq!(
        recovered.read().unwrap().get(b"first").unwrap().is_some(),
        transaction == 2
    );
    assert_eq!(
        recovered.read().unwrap().get(b"second").unwrap().is_some(),
        transaction == 2
    );
}

#[cfg(feature = "crash-injection")]
#[test]
fn crash_after_shared_sync_recovers_the_complete_group() {
    let file = TestFile::new("group-crash");
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("group_commit_crash_worker")
        .arg("--nocapture")
        .env("WSLDB_GROUP_CRASH_PATH", file.path())
        .env("WSLDB_FAILPOINT", "write_buffer_sync")
        .env("WSLDB_FAIL_AFTER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "group sync failpoint was not reached");

    let recovered = Database::open(file.path()).unwrap();
    assert_eq!(recovered.stats().unwrap().transaction_id.value(), 2);
    assert_eq!(recovered.verify().unwrap().entries, 2);
    assert_eq!(
        recovered.read().unwrap().get(b"first").unwrap(),
        Some(b"one".to_vec())
    );
    assert_eq!(
        recovered.read().unwrap().get(b"second").unwrap(),
        Some(b"two".to_vec())
    );
}

#[cfg(feature = "crash-injection")]
#[test]
fn group_commit_crash_worker() {
    let Some(path) = std::env::var_os("WSLDB_GROUP_CRASH_PATH") else {
        return;
    };
    let database =
        Database::open_with_options(path, DatabaseOptions::new().write_buffer_bytes(1)).unwrap();
    let mut first = WriteBatch::new();
    first.put(b"first", b"one").unwrap();
    let mut second = WriteBatch::new();
    second.put(b"second", b"two").unwrap();
    database.commit_group(vec![first, second]).unwrap();
}

fn buffered(file: &TestFile, threshold: u64) -> Database {
    Database::open_with_options(
        file.path(),
        DatabaseOptions::new().write_buffer_bytes(threshold),
    )
    .unwrap()
}
