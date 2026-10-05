use super::super::{
    HEADER_BYTES, WalRecord, WalRecordKind, append_group, encode, group as group_codec, load,
};
use super::{repair_checksums, temporary_file};
use crate::domain::ChangeSet;
use crate::infrastructure::positioned_io::write_all_at;
use crate::{Error, MAX_GROUP_TRANSACTIONS, TransactionId};

#[test]
fn compact_group_round_trip_preserves_logical_records() {
    let records = records();
    let compact = group_codec::encode(&records).unwrap();
    let decoded = group_codec::decode(&compact).unwrap();
    let legacy_bytes = records
        .iter()
        .map(|record| {
            encode(
                record.base_transaction_id,
                record.transaction_id,
                record.entry_count,
                &record.changes,
            )
            .unwrap()
            .len()
        })
        .sum::<usize>();

    assert!(compact.len() < legacy_bytes);
    assert_eq!(decoded.len(), records.len());
    for (actual, expected) in decoded.iter().zip(&records) {
        assert_eq!(actual.kind, WalRecordKind::Transaction);
        assert_eq!(actual.base_transaction_id, expected.base_transaction_id);
        assert_eq!(actual.transaction_id, expected.transaction_id);
        assert_eq!(actual.entry_count, expected.entry_count);
        assert_eq!(changes(actual), changes(expected));
    }
}

#[test]
fn every_truncated_group_is_an_incomplete_tail_without_a_prefix() {
    let bytes = group_codec::encode(&records()).unwrap();
    let (path, file) = temporary_file("truncated-group");

    for end in 1..bytes.len() {
        file.set_len(0).unwrap();
        write_all_at(&file, &bytes[..end], 0).unwrap();
        let loaded = load(&file).unwrap();
        assert!(loaded.records.is_empty(), "end={end}");
        assert_eq!(loaded.valid_bytes, 0, "end={end}");
        assert!(loaded.has_incomplete_tail, "end={end}");
    }

    drop(file);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn changed_complete_group_fails_closed() {
    let mut bytes = group_codec::encode(&records()).unwrap();
    bytes[HEADER_BYTES + 4] ^= 1;

    assert!(matches!(
        group_codec::decode(&bytes),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn compact_group_rejects_noncanonical_and_overflowing_varints() {
    let mut noncanonical = group_codec::encode(&[empty_record(128)]).unwrap();
    noncanonical[HEADER_BYTES..HEADER_BYTES + 2].copy_from_slice(&[0x81, 0]);
    repair_checksums(&mut noncanonical);
    assert!(matches!(
        group_codec::decode(&noncanonical),
        Err(Error::Corrupt(_))
    ));

    let mut overflowing = group_codec::encode(&records()).unwrap();
    overflowing[HEADER_BYTES..HEADER_BYTES + 10]
        .copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2]);
    repair_checksums(&mut overflowing);
    assert!(matches!(
        group_codec::decode(&overflowing),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn compact_group_rejects_inconsistent_summary_fields() {
    let encoded = group_codec::encode(&records()).unwrap();
    for mutation in [
        HeaderMutation::FinalTransaction,
        HeaderMutation::FinalEntries,
    ] {
        let mut changed = encoded.clone();
        match mutation {
            HeaderMutation::FinalTransaction => {
                changed[24..32].copy_from_slice(&9_u64.to_le_bytes());
            }
            HeaderMutation::FinalEntries => {
                changed[32..40].copy_from_slice(&9_u64.to_le_bytes());
            }
        }
        repair_checksums(&mut changed);
        assert!(matches!(
            group_codec::decode(&changed),
            Err(Error::Corrupt(_))
        ));
    }

    let mut wrong_count = encoded;
    wrong_count[24..32].copy_from_slice(&4_u64.to_le_bytes());
    wrong_count[40..44].copy_from_slice(&4_u32.to_le_bytes());
    repair_checksums(&mut wrong_count);
    assert!(matches!(
        group_codec::decode(&wrong_count),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn compact_group_rejects_a_transaction_count_above_the_resource_limit() {
    let mut encoded = group_codec::encode(&records()).unwrap();
    let impossible = u32::try_from(MAX_GROUP_TRANSACTIONS + 1).unwrap();
    encoded[40..44].copy_from_slice(&impossible.to_le_bytes());
    repair_checksums(&mut encoded);

    assert!(matches!(
        group_codec::decode(&encoded),
        Err(Error::Corrupt(message))
            if message == "write-ahead group exceeds the transaction limit"
    ));
}

#[test]
fn group_validation_happens_before_any_write() {
    let (path, file) = temporary_file("invalid-compact-group");
    assert!(matches!(
        append_group(&file, 0, &[]),
        Err(Error::InternalInvariant(_))
    ));

    let mut snapshot = empty_record(1);
    snapshot.kind = WalRecordKind::Snapshot;
    assert!(matches!(
        append_group(&file, 0, &[snapshot]),
        Err(Error::InternalInvariant(_))
    ));
    assert_eq!(file.metadata().unwrap().len(), 0);

    drop(file);
    std::fs::remove_file(path).unwrap();
}

fn records() -> Vec<WalRecord> {
    let mut first = ChangeSet::new();
    first.insert(b"shared-prefix-alpha", Some(b"one".to_vec()));
    first.insert(b"shared-prefix-beta", Some(Vec::new()));
    let second = ChangeSet::new();
    let mut third = ChangeSet::new();
    third.insert(b"shared-prefix-delta", None);
    third.insert(b"shared-prefix-gamma", Some(b"three".to_vec()));
    vec![
        WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(0),
            transaction_id: TransactionId::from_raw(1),
            entry_count: 2,
            changes: first,
        },
        WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(1),
            transaction_id: TransactionId::from_raw(2),
            entry_count: 2,
            changes: second,
        },
        WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(2),
            transaction_id: TransactionId::from_raw(3),
            entry_count: 3,
            changes: third,
        },
    ]
}

fn empty_record(entry_count: u64) -> WalRecord {
    WalRecord {
        kind: WalRecordKind::Transaction,
        base_transaction_id: TransactionId::from_raw(0),
        transaction_id: TransactionId::from_raw(1),
        entry_count,
        changes: ChangeSet::new(),
    }
}

fn changes(record: &WalRecord) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    record
        .changes
        .sorted_iter()
        .map(|(key, value)| (key.to_vec(), value.map(<[u8]>::to_vec)))
        .collect()
}

#[derive(Clone, Copy)]
enum HeaderMutation {
    FinalTransaction,
    FinalEntries,
}
