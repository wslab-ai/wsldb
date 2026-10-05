use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    HEADER_BYTES, WalRecord, WalRecordKind, allocate_record_buffer, append, append_group, decode,
    encode, encode_record, load, replay_after,
};
#[cfg(unix)]
use super::{is_current, open, replace_with_snapshot_streamed};
use crate::domain::{ChangeSet, Head};
use crate::infrastructure::checksum::crc32c;
use crate::infrastructure::positioned_io::{read_exact_at, write_all_at};
#[cfg(unix)]
use crate::infrastructure::write_buffer::WriteBuffer;
use crate::{Error, TransactionId};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

mod group;

#[test]
fn record_round_trip_is_deterministic() {
    let mut changes = ChangeSet::new();
    changes.insert(b"third", Some(b"value".to_vec()));
    changes.insert(b"first", None);
    changes.insert(b"second", Some(vec![7; 5000]));
    let base = TransactionId::from_raw(8);
    let transaction = TransactionId::from_raw(9);

    let encoded = encode(base, transaction, 17, &changes).unwrap();
    let decoded = decode(&encoded).unwrap();

    assert_eq!(decoded.base_transaction_id.value(), 8);
    assert_eq!(decoded.kind, WalRecordKind::Transaction);
    assert_eq!(decoded.transaction_id.value(), 9);
    assert_eq!(decoded.entry_count, 17);
    assert_eq!(
        decoded
            .changes
            .sorted_iter()
            .map(|(key, value)| (key.to_vec(), value.map(<[u8]>::to_vec)))
            .collect::<Vec<_>>(),
        vec![
            (b"first".to_vec(), None),
            (b"second".to_vec(), Some(vec![7; 5000])),
            (b"third".to_vec(), Some(b"value".to_vec())),
        ]
    );
    assert_eq!(
        encode(
            decoded.base_transaction_id,
            decoded.transaction_id,
            decoded.entry_count,
            &decoded.changes,
        )
        .unwrap(),
        encoded
    );
}

#[test]
fn record_buffer_capacity_overflow_returns_a_typed_error() {
    assert!(matches!(
        allocate_record_buffer(usize::MAX),
        Err(Error::FileTooLarge)
    ));
}

#[test]
fn snapshot_record_collapses_a_transaction_range() {
    let mut changes = ChangeSet::new();
    changes.insert(b"deleted", None);
    changes.insert(b"key", Some(b"latest".to_vec()));
    let bytes = encode_record(
        WalRecordKind::Snapshot,
        TransactionId::from_raw(3),
        TransactionId::from_raw(19),
        7,
        &changes,
    )
    .unwrap();

    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.kind, WalRecordKind::Snapshot);
    assert_eq!(decoded.base_transaction_id.value(), 3);
    assert_eq!(decoded.transaction_id.value(), 19);
    assert_eq!(decoded.entry_count, 7);
}

#[test]
fn prefix_encoding_round_trips_zero_partial_full_and_maximum_prefixes() {
    let mut changes = ChangeSet::new();
    changes.insert(b"alpha", Some(b"one".to_vec()));
    changes.insert(b"alphabet", Some(b"two".to_vec()));
    changes.insert(b"alpine", None);
    changes.insert(b"zulu", Some(b"last".to_vec()));
    let mut long = vec![b'a'; 1_024];
    long[0] = b'y';
    changes.insert(&long, Some(b"maximum".to_vec()));
    let base = TransactionId::from_raw(7);
    let transaction = TransactionId::from_raw(8);

    let encoded = encode(base, transaction, 4, &changes).unwrap();
    let decoded = decode(&encoded).unwrap();

    assert_eq!(
        decoded
            .changes
            .sorted_iter()
            .map(|(key, value)| (key.to_vec(), value.map(<[u8]>::to_vec)))
            .collect::<Vec<_>>(),
        changes
            .sorted_iter()
            .map(|(key, value)| (key.to_vec(), value.map(<[u8]>::to_vec)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        encode(base, transaction, 4, &decoded.changes).unwrap(),
        encoded
    );
}

#[test]
fn prefix_encoding_rejects_a_prefix_longer_than_the_previous_key() {
    let mut changes = ChangeSet::new();
    changes.insert(b"alpha", Some(b"one".to_vec()));
    changes.insert(b"alphabet", Some(b"two".to_vec()));
    let mut encoded = encode(
        TransactionId::from_raw(7),
        TransactionId::from_raw(8),
        2,
        &changes,
    )
    .unwrap();
    let second = HEADER_BYTES + 4 + b"alpha".len() + 8 + b"one".len();
    encoded[second..second + 2].copy_from_slice(&6_u16.to_le_bytes());
    repair_checksums(&mut encoded);

    assert!(matches!(decode(&encoded), Err(Error::Corrupt(_))));
}

#[test]
fn record_rejects_a_mutation_count_larger_than_its_payload() {
    let (base, transaction, changes) = record();
    let mut encoded = encode(base, transaction, 1, &changes).unwrap();
    encoded[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    repair_checksums(&mut encoded);

    assert!(matches!(
        decode(&encoded),
        Err(Error::Corrupt(message))
            if message == "write-ahead mutation count exceeds its payload"
    ));
}

#[test]
fn group_append_preserves_every_transaction_record() {
    let (path, file) = temporary_file("group-append");
    let mut first = ChangeSet::new();
    first.insert(b"first", Some(b"one".to_vec()));
    let mut second = ChangeSet::new();
    second.insert(b"second", Some(b"two".to_vec()));
    let records = [
        WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(0),
            transaction_id: TransactionId::from_raw(1),
            entry_count: 1,
            changes: first,
        },
        WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(1),
            transaction_id: TransactionId::from_raw(2),
            entry_count: 2,
            changes: second,
        },
    ];

    let appended = append_group(&file, 0, &records).unwrap();
    let loaded = load(&file).unwrap();

    assert_eq!(loaded.valid_bytes, appended.end);
    assert_eq!(loaded.records.len(), 2);
    assert_eq!(loaded.records[0].transaction_id.value(), 1);
    assert_eq!(loaded.records[1].transaction_id.value(), 2);
    drop(file);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn group_append_rejects_a_discontinuous_chain_before_writing() {
    let (path, file) = temporary_file("discontinuous-group");
    let (_, _, changes) = record();
    let records = [
        WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(0),
            transaction_id: TransactionId::from_raw(1),
            entry_count: 1,
            changes: changes.clone(),
        },
        WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(3),
            transaction_id: TransactionId::from_raw(4),
            entry_count: 2,
            changes,
        },
    ];

    assert!(matches!(
        append_group(&file, 0, &records),
        Err(Error::InternalInvariant(_))
    ));
    assert_eq!(file.metadata().unwrap().len(), 0);
    drop(file);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn replay_rejects_a_snapshot_after_a_transaction() {
    let (path, file) = temporary_file("late-snapshot");
    let (base, transaction, changes) = record();
    let first = append(&file, 0, base, transaction, 1, &changes).unwrap();
    let snapshot = encode_record(
        WalRecordKind::Snapshot,
        transaction,
        TransactionId::from_raw(9),
        1,
        &changes,
    )
    .unwrap();
    write_all_at(&file, &snapshot, first.end).unwrap();
    file.sync_all().unwrap();

    assert!(matches!(load(&file), Err(Error::Corrupt(_))));
    drop(file);
    std::fs::remove_file(path).unwrap();
}

#[cfg(unix)]
#[test]
fn snapshot_replacement_is_atomic_and_changes_the_open_generation() {
    let database = temporary_path("snapshot-replacement");
    let old = open(&database).unwrap();
    let (_, _, changes) = record();
    let first = append(
        &old,
        0,
        TransactionId::from_raw(0),
        TransactionId::from_raw(1),
        1,
        &changes,
    )
    .unwrap();
    append(
        &old,
        first.end,
        TransactionId::from_raw(1),
        TransactionId::from_raw(2),
        1,
        &changes,
    )
    .unwrap();

    let mut buffer = WriteBuffer::empty(Head::empty());
    buffer
        .apply(WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(0),
            transaction_id: TransactionId::from_raw(1),
            entry_count: 1,
            changes: changes.clone(),
        })
        .unwrap();
    buffer
        .apply(WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(1),
            transaction_id: TransactionId::from_raw(2),
            entry_count: 1,
            changes,
        })
        .unwrap();
    let replacement = replace_with_snapshot_streamed(
        &database,
        TransactionId::from_raw(0),
        TransactionId::from_raw(2),
        1,
        &buffer,
    )
    .unwrap()
    .unwrap();

    assert!(!is_current(&old, &database).unwrap());
    assert!(is_current(&replacement.file, &database).unwrap());
    let loaded = load(&replacement.file).unwrap();
    assert_eq!(loaded.records.len(), 1);
    assert_eq!(loaded.records[0].kind, WalRecordKind::Snapshot);
    drop(old);
    drop(replacement.file);
    std::fs::remove_file(super::path(&database)).unwrap();
}

#[cfg(unix)]
#[test]
fn streamed_snapshot_matches_canonical_bytes_across_stage_boundaries() {
    const STAGE_BYTES: usize = 256 * 1024;

    let database = temporary_path("streamed-snapshot-bytes");
    let mut buffer = WriteBuffer::empty(Head::empty());
    let mut first = ChangeSet::new();
    first.insert(b"deleted", Some(b"old".to_vec()));
    first.insert(b"first", Some(b"initial".to_vec()));
    first.insert(b"large", Some(vec![0xa5; STAGE_BYTES + 17]));
    buffer
        .apply(WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(0),
            transaction_id: TransactionId::from_raw(1),
            entry_count: 3,
            changes: first,
        })
        .unwrap();
    let mut second = ChangeSet::new();
    second.insert(b"deleted", None);
    second.insert(b"first", Some(b"latest".to_vec()));
    second.insert(b"new", Some(b"value".to_vec()));
    buffer
        .apply(WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(1),
            transaction_id: TransactionId::from_raw(2),
            entry_count: 3,
            changes: second,
        })
        .unwrap();

    let expected_changes = buffer.latest_changes().unwrap();
    let expected = encode_record(
        WalRecordKind::Snapshot,
        TransactionId::from_raw(0),
        TransactionId::from_raw(2),
        3,
        &expected_changes,
    )
    .unwrap();
    let replacement = replace_with_snapshot_streamed(
        &database,
        TransactionId::from_raw(0),
        TransactionId::from_raw(2),
        3,
        &buffer,
    )
    .unwrap()
    .unwrap();

    assert_eq!(std::fs::read(super::path(&database)).unwrap(), expected);
    assert_eq!(replacement.bytes, expected.len() as u64);
    let loaded = load(&replacement.file).unwrap();
    assert_eq!(loaded.records.len(), 1);
    assert_eq!(loaded.records[0].changes.len(), expected_changes.len());
    drop(replacement.file);
    std::fs::remove_file(super::path(&database)).unwrap();
}

#[test]
fn every_truncated_record_is_rejected() {
    let (base, transaction, changes) = record();
    let encoded = encode(base, transaction, 1, &changes).unwrap();
    for end in 0..encoded.len() {
        assert!(decode(&encoded[..end]).is_err(), "end={end}");
    }
}

#[test]
fn changed_complete_record_fails_closed() {
    let (base, transaction, changes) = record();
    let mut encoded = encode(base, transaction, 1, &changes).unwrap();
    encoded[52] ^= 1;
    assert!(matches!(decode(&encoded), Err(Error::Corrupt(_))));
}

#[test]
fn invalid_magic_fails_closed() {
    let (base, transaction, changes) = record();
    let mut encoded = encode(base, transaction, 1, &changes).unwrap();
    encoded[0] ^= 1;
    assert!(matches!(decode(&encoded), Err(Error::Corrupt(_))));
}

#[test]
fn load_keeps_a_valid_prefix_before_an_incomplete_tail() {
    let (path, file) = temporary_file("truncated-tail");
    let (_, _, changes) = record();
    let first = append(
        &file,
        0,
        TransactionId::from_raw(0),
        TransactionId::from_raw(1),
        1,
        &changes,
    )
    .unwrap();
    let second = append(
        &file,
        first.end,
        TransactionId::from_raw(1),
        TransactionId::from_raw(2),
        2,
        &changes,
    )
    .unwrap();
    file.set_len(second.end - 1).unwrap();

    let loaded = load(&file).unwrap();
    assert_eq!(loaded.records.len(), 1);
    assert_eq!(loaded.valid_bytes, first.end);
    assert!(loaded.has_incomplete_tail);
    drop(file);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn load_rejects_a_corrupt_complete_tail() {
    let (path, file) = temporary_file("corrupt-tail");
    let (_, _, changes) = record();
    let first = append(
        &file,
        0,
        TransactionId::from_raw(0),
        TransactionId::from_raw(1),
        1,
        &changes,
    )
    .unwrap();
    append(
        &file,
        first.end,
        TransactionId::from_raw(1),
        TransactionId::from_raw(2),
        2,
        &changes,
    )
    .unwrap();
    let offset = first.end + HEADER_BYTES as u64;
    let mut byte = [0];
    read_exact_at(&file, &mut byte, offset).unwrap();
    byte[0] ^= 1;
    write_all_at(&file, &byte, offset).unwrap();

    assert!(matches!(load(&file), Err(Error::Corrupt(_))));
    drop(file);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn load_rejects_a_discontinuous_transaction_chain() {
    let (path, file) = temporary_file("transaction-gap");
    let (_, _, changes) = record();
    let first = append(
        &file,
        0,
        TransactionId::from_raw(0),
        TransactionId::from_raw(1),
        1,
        &changes,
    )
    .unwrap();
    append(
        &file,
        first.end,
        TransactionId::from_raw(4),
        TransactionId::from_raw(5),
        2,
        &changes,
    )
    .unwrap();

    assert!(matches!(load(&file), Err(Error::Corrupt(_))));
    drop(file);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn tail_replay_starts_at_a_validated_transaction_boundary() {
    let (path, file) = temporary_file("tail-replay");
    let (_, _, changes) = record();
    let first = append(
        &file,
        0,
        TransactionId::from_raw(0),
        TransactionId::from_raw(1),
        1,
        &changes,
    )
    .unwrap();
    let second = append(
        &file,
        first.end,
        TransactionId::from_raw(1),
        TransactionId::from_raw(2),
        2,
        &changes,
    )
    .unwrap();
    let mut transactions = Vec::new();

    let scan = replay_after(&file, first.end, TransactionId::from_raw(1), |record| {
        transactions.push(record.transaction_id.value());
        Ok(())
    })
    .unwrap();

    assert_eq!(transactions, [2]);
    assert_eq!(scan.valid_bytes, second.end);
    assert!(!scan.has_incomplete_tail);
    assert!(matches!(
        replay_after(&file, first.end, TransactionId::from_raw(9), |_| Ok(())),
        Err(Error::Corrupt(_))
    ));
    drop(file);
    std::fs::remove_file(path).unwrap();
}

fn record() -> (TransactionId, TransactionId, ChangeSet) {
    let mut changes = ChangeSet::new();
    changes.insert(b"key", Some(b"value".to_vec()));
    (
        TransactionId::from_raw(3),
        TransactionId::from_raw(4),
        changes,
    )
}

fn repair_checksums(bytes: &mut [u8]) {
    let footer = bytes.len() - 16;
    let payload_crc = crc32c(&bytes[HEADER_BYTES..footer]);
    bytes[44..48].copy_from_slice(&payload_crc.to_le_bytes());
    let header_crc = crc32c(&bytes[..48]);
    bytes[48..52].copy_from_slice(&header_crc.to_le_bytes());
    let record_crc = crc32c(&bytes[..footer]);
    bytes[footer..footer + 4].copy_from_slice(&record_crc.to_le_bytes());
}

fn temporary_file(label: &str) -> (PathBuf, File) {
    let path = temporary_path(label);
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    (path, file)
}

fn temporary_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "wsldb-wal-{label}-{}-{}",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ))
}
