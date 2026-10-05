use std::sync::Arc;

use super::{PointIndex, PointIndexSlot, WriteBuffer};
use crate::TransactionId;
use crate::domain::{ChangeSet, Head};
use crate::infrastructure::membership_filter::GrowingMembershipFilter;
use crate::infrastructure::wal::{WalLoad, WalRecord, WalRecordKind};

#[test]
fn old_snapshots_see_their_own_key_versions() {
    let checkpoint = Head::empty();
    let mut buffer = empty_buffer(checkpoint);
    buffer.apply(record(0, 1, b"one", 1)).unwrap();
    let first = buffer.head();
    buffer.apply(record(1, 2, b"two", 1)).unwrap();
    let second = buffer.head();

    assert_eq!(
        buffer.visible_value(first, b"key").unwrap(),
        Some(Some(b"one".to_vec()))
    );
    assert_eq!(
        buffer.visible_value(second, b"key").unwrap(),
        Some(Some(b"two".to_vec()))
    );
}

#[test]
fn point_index_preserves_historical_versions() {
    let mut buffer = empty_buffer(Head::empty());
    let index = PointIndex::from_buffer(&buffer).unwrap();
    index.apply_durable(buffer.apply(record(0, 1, b"one", 1)).unwrap());
    index.apply_durable(buffer.apply(record(1, 2, b"two", 1)).unwrap());

    assert_eq!(
        index
            .visible_value(TransactionId::from_raw(1), b"key")
            .unwrap(),
        Some(Some(b"one".to_vec()))
    );
    assert_eq!(
        index
            .visible_value(TransactionId::from_raw(2), b"key")
            .unwrap(),
        Some(Some(b"two".to_vec()))
    );
}

#[test]
fn fallible_point_index_slot_reuses_one_initialized_index() {
    let mut buffer = empty_buffer(Head::empty());
    buffer.apply(record(0, 1, b"one", 1)).unwrap();
    let slot = PointIndexSlot::new();

    let initialization = slot.lock_if_empty().unwrap().unwrap();
    assert!(slot.get().is_none());
    drop(initialization);
    assert!(matches!(
        slot.get_or_initialize(|| Err(crate::Error::FileTooLarge)),
        Err(crate::Error::FileTooLarge)
    ));
    assert!(slot.get().is_none());

    let first = slot
        .get_or_initialize(|| PointIndex::from_buffer(&buffer))
        .unwrap() as *const PointIndex;
    let second = slot
        .get_or_initialize(|| PointIndex::from_buffer(&buffer))
        .unwrap() as *const PointIndex;

    assert_eq!(first, second);
    assert!(slot.get().unwrap().capacity_for(b"key").unwrap() >= 1);
}

#[test]
fn apply_reserves_version_storage_before_publishing_records() {
    let mut buffer = empty_buffer(Head::empty());
    let mut changes = ChangeSet::new();
    changes.insert(b"first", Some(b"one".to_vec()));
    changes.insert(b"second", Some(b"two-two".to_vec()));
    let record = WalRecord {
        kind: WalRecordKind::Transaction,
        base_transaction_id: TransactionId::from_raw(0),
        transaction_id: TransactionId::from_raw(1),
        entry_count: 2,
        changes,
    };

    assert_eq!(
        buffer
            .prepare_apply(std::iter::once(&record.changes))
            .unwrap(),
        2
    );
    let prepared = buffer.arena.capacities().unwrap();
    buffer.apply(record).unwrap();

    assert_eq!(buffer.arena.capacities().unwrap(), prepared);
}

#[test]
fn prepared_ordered_nodes_are_invisible_and_reused_at_publication() {
    let filter = GrowingMembershipFilter::new(64).unwrap();
    let mut buffer = WriteBuffer::empty_with_membership_filter(Head::empty(), filter);
    let pending = record(0, 1, b"one", 1);
    let shared_key = pending.changes.clone().into_sorted_iter().next().unwrap().0;

    buffer
        .prepare_apply(std::iter::once(&pending.changes))
        .unwrap();

    assert_eq!(buffer.versions.len(), 1);
    assert_eq!(buffer.versioned_keys, 0);
    assert!(!buffer.contains_key(b"key"));
    assert_eq!(buffer.visible_value(Head::empty(), b"key").unwrap(), None);
    assert!(
        buffer
            .merge_scan(Head::empty(), Vec::new())
            .unwrap()
            .is_empty()
    );
    assert!(
        buffer
            .merge_range(Head::empty(), Vec::new(), b"", false, None, 8)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        buffer
            .visible_range_changes(Head::empty(), b"", false, None)
            .unwrap(),
        0
    );
    assert!(buffer.latest_changes().unwrap().is_empty());
    assert!(buffer.latest_are_puts().unwrap());
    let mut visited = false;
    buffer
        .visit_latest(|_, _| {
            visited = true;
            Ok(())
        })
        .unwrap();
    assert!(!visited);
    assert_eq!(buffer.key_filter_may_contain(b"key"), Some(false));
    assert!(
        PointIndex::from_buffer(&buffer)
            .unwrap()
            .visible_value(TransactionId::from_raw(0), b"key")
            .unwrap()
            .is_none()
    );

    let prepared_nodes = buffer.versions.len();
    let mut updates = Vec::with_capacity(1);
    buffer.apply_durable(pending, Some(&mut updates));

    assert_eq!(buffer.versions.len(), prepared_nodes);
    assert_eq!(buffer.versioned_keys, 1);
    assert!(buffer.contains_key(b"key"));
    assert_eq!(
        buffer.visible_value(buffer.head(), b"key").unwrap(),
        Some(Some(b"one".to_vec()))
    );
    assert_eq!(buffer.key_filter_may_contain(b"key"), Some(true));
    assert!(Arc::ptr_eq(
        &shared_key,
        buffer.versions.first_key_value().unwrap().0
    ));
    assert!(Arc::ptr_eq(&shared_key, &updates[0].key));
}

#[test]
fn failed_append_preparation_can_be_discarded_without_logical_changes() {
    let mut buffer = empty_buffer(Head::empty());
    let pending = record(0, 1, b"one", 1);

    buffer
        .prepare_apply(std::iter::once(&pending.changes))
        .unwrap();
    buffer.discard_prepared();

    assert!(buffer.versions.is_empty());
    assert_eq!(buffer.versioned_keys, 0);
    assert_eq!(buffer.head(), Head::empty());
    assert_eq!(buffer.visible_value(Head::empty(), b"key").unwrap(), None);

    buffer.apply(pending).unwrap();
    assert_eq!(
        buffer.visible_value(buffer.head(), b"key").unwrap(),
        Some(Some(b"one".to_vec()))
    );
}

#[test]
fn durable_apply_reuses_the_change_set_key_allocation() {
    let mut buffer = empty_buffer(Head::empty());
    let pending = record(0, 1, b"one", 1);
    let shared_key = pending.changes.clone().into_sorted_iter().next().unwrap().0;

    let updates = buffer.apply(pending).unwrap();
    let stored_key = buffer.versions.first_key_value().unwrap().0;

    assert!(Arc::ptr_eq(&shared_key, stored_key));
    assert!(Arc::ptr_eq(&shared_key, &updates[0].key));
}

#[test]
fn snapshot_rebase_shares_keys_and_arena_but_not_the_ordered_map() {
    let checkpoint = Head::empty();
    let mut original = empty_buffer(checkpoint);
    original.apply(record(0, 1, b"one", 1)).unwrap();
    let original_head = original.head();
    let rebased_checkpoint = Head::new(
        checkpoint.transaction_id(),
        checkpoint.root(),
        checkpoint.last_page(),
        checkpoint.entry_count(),
    );
    let filter = GrowingMembershipFilter::new(64).unwrap();
    let mut rebased = original
        .snapshot_rebased(rebased_checkpoint, filter, 128)
        .unwrap();

    assert!(Arc::ptr_eq(&original.arena, &rebased.arena));
    assert!(Arc::ptr_eq(
        original.versions.first_key_value().unwrap().0,
        rebased.versions.first_key_value().unwrap().0,
    ));
    rebased.apply(record(1, 2, b"two", 1)).unwrap();
    assert_eq!(
        original.visible_value(original_head, b"key").unwrap(),
        Some(Some(b"one".to_vec()))
    );
    assert_eq!(
        rebased.visible_value(rebased.head(), b"key").unwrap(),
        Some(Some(b"two".to_vec()))
    );
}

#[test]
fn point_index_disagreement_is_rejected_before_durable_apply() {
    let mut buffer = empty_buffer(Head::empty());
    buffer.apply(record(0, 1, b"one", 1)).unwrap();
    let index = PointIndex::empty_for(&buffer);
    let mut changes = ChangeSet::new();
    changes.insert(b"key", Some(b"two".to_vec()));

    index.preflight_one(b"key", false).unwrap();
    assert!(matches!(
        index.preflight_one(b"key", true),
        Err(crate::Error::InternalInvariant(_))
    ));
    assert!(matches!(
        index.preflight(&changes, &buffer),
        Err(crate::Error::InternalInvariant(_))
    ));
}

#[test]
fn point_index_preflight_reserves_new_key_capacity() {
    let mut buffer = empty_buffer(Head::empty());
    let index = PointIndex::empty_for(&buffer);
    let mut changes = ChangeSet::new();
    changes.insert(b"key", Some(b"value".to_vec()));
    let record = WalRecord {
        kind: WalRecordKind::Transaction,
        base_transaction_id: TransactionId::from_raw(0),
        transaction_id: TransactionId::from_raw(1),
        entry_count: 1,
        changes,
    };

    index.preflight(&record.changes, &buffer).unwrap();
    let prepared = index.capacity_for(b"key").unwrap();
    index.apply_durable(buffer.apply(record).unwrap());

    assert_eq!(index.capacity_for(b"key").unwrap(), prepared);
}

#[test]
fn key_filter_tracks_replay_and_new_generations_start_empty() {
    let filter = GrowingMembershipFilter::new(64).unwrap();
    let mut buffer = WriteBuffer::empty_with_membership_filter(Head::empty(), filter);
    assert_eq!(buffer.key_filter_may_contain(b"key"), Some(false));

    buffer.replay(record(0, 1, b"value", 1)).unwrap();
    assert_eq!(buffer.key_filter_may_contain(b"key"), Some(true));
    assert_eq!(
        buffer.visible_presence(buffer.head(), b"key").unwrap(),
        Some(true)
    );

    let filter = GrowingMembershipFilter::new(64).unwrap();
    let next = WriteBuffer::empty_with_membership_filter(buffer.head(), filter);
    assert_eq!(next.key_filter_may_contain(b"key"), Some(false));
}

#[test]
fn guarded_key_filter_grows_only_after_crossing_capacity() {
    let filter = GrowingMembershipFilter::new(1).unwrap();
    let mut buffer = WriteBuffer::empty_with_membership_filter(Head::empty(), filter);
    assert!(!buffer.key_filter_needs_growth(1).unwrap());
    buffer.apply(record(0, 1, b"one", 1)).unwrap();
    assert_eq!(buffer.key_filter_capacity(), Some(1));

    buffer.apply(record(1, 2, b"two", 1)).unwrap();
    assert_eq!(buffer.key_filter_capacity(), Some(1));
    assert!(buffer.key_filter_needs_growth(1).unwrap());

    let mut changes = ChangeSet::new();
    changes.insert(b"second", Some(b"value".to_vec()));
    buffer
        .apply(WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(2),
            transaction_id: TransactionId::from_raw(3),
            entry_count: 2,
            changes,
        })
        .unwrap();
    assert_eq!(buffer.key_filter_capacity(), Some(2));
    assert_eq!(buffer.key_filter_may_contain(b"key"), Some(true));
    assert_eq!(buffer.key_filter_may_contain(b"second"), Some(true));
}

#[test]
fn scan_merge_applies_inserts_updates_and_tombstones() {
    let checkpoint = Head::new(TransactionId::from_raw(4), None, 1, 2);
    let mut buffer = empty_buffer(checkpoint);
    let mut changes = ChangeSet::new();
    changes.insert(b"a", None);
    changes.insert(b"b", Some(b"new".to_vec()));
    changes.insert(b"c", Some(b"three".to_vec()));
    buffer
        .apply(WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(4),
            transaction_id: TransactionId::from_raw(5),
            entry_count: 2,
            changes,
        })
        .unwrap();
    let first = buffer.head();

    assert_eq!(
        buffer
            .merge_scan(
                first,
                vec![
                    (b"a".to_vec(), b"one".to_vec()),
                    (b"b".to_vec(), b"two".to_vec()),
                ],
            )
            .unwrap(),
        vec![
            (b"b".to_vec(), b"new".to_vec()),
            (b"c".to_vec(), b"three".to_vec()),
        ]
    );

    let mut changes = ChangeSet::new();
    changes.insert(b"b", Some(b"newest".to_vec()));
    changes.insert(b"c", None);
    changes.insert(b"d", Some(b"four".to_vec()));
    buffer
        .apply(WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(5),
            transaction_id: TransactionId::from_raw(6),
            entry_count: 2,
            changes,
        })
        .unwrap();

    assert_eq!(
        buffer
            .merge_scan(
                first,
                vec![
                    (b"a".to_vec(), b"one".to_vec()),
                    (b"b".to_vec(), b"two".to_vec()),
                ],
            )
            .unwrap(),
        vec![
            (b"b".to_vec(), b"new".to_vec()),
            (b"c".to_vec(), b"three".to_vec()),
        ]
    );
    assert_eq!(
        buffer
            .merge_scan(
                buffer.head(),
                vec![
                    (b"a".to_vec(), b"one".to_vec()),
                    (b"b".to_vec(), b"two".to_vec()),
                ],
            )
            .unwrap(),
        vec![
            (b"b".to_vec(), b"newest".to_vec()),
            (b"d".to_vec(), b"four".to_vec()),
        ]
    );
}

#[test]
fn range_merge_applies_visible_changes_in_order_before_the_limit() {
    let checkpoint = Head::new(TransactionId::from_raw(4), None, 1, 4);
    let mut buffer = empty_buffer(checkpoint);
    let mut changes = ChangeSet::new();
    changes.insert(b"a", None);
    changes.insert(b"b", Some(b"new-two".to_vec()));
    changes.insert(b"c", Some(b"three".to_vec()));
    changes.insert(b"e", Some(b"five".to_vec()));
    changes.insert(b"f", None);
    buffer
        .apply(WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id: TransactionId::from_raw(4),
            transaction_id: TransactionId::from_raw(5),
            entry_count: 4,
            changes,
        })
        .unwrap();
    let base = || {
        vec![
            (b"a".to_vec(), b"one".to_vec()),
            (b"b".to_vec(), b"two".to_vec()),
            (b"d".to_vec(), b"four".to_vec()),
            (b"f".to_vec(), b"six".to_vec()),
        ]
    };

    assert_eq!(
        buffer
            .merge_range(buffer.head(), base(), b"a", false, None, 3)
            .unwrap(),
        vec![
            (b"b".to_vec(), b"new-two".to_vec()),
            (b"c".to_vec(), b"three".to_vec()),
            (b"d".to_vec(), b"four".to_vec()),
        ]
    );
    assert_eq!(
        buffer
            .merge_range(checkpoint, base(), b"a", false, None, 3)
            .unwrap(),
        base().into_iter().take(3).collect::<Vec<_>>()
    );
    assert_eq!(
        buffer
            .merge_range(buffer.head(), Vec::new(), b"c", false, Some(b"f"), 3)
            .unwrap(),
        vec![
            (b"c".to_vec(), b"three".to_vec()),
            (b"e".to_vec(), b"five".to_vec()),
        ]
    );
    assert_eq!(
        buffer
            .merge_range(buffer.head(), Vec::new(), b"c", true, Some(b"f"), 3)
            .unwrap(),
        vec![(b"e".to_vec(), b"five".to_vec())]
    );
}

fn empty_buffer(checkpoint: Head) -> WriteBuffer {
    WriteBuffer::from_load(
        checkpoint,
        WalLoad {
            records: Vec::new(),
            valid_bytes: 0,
            has_incomplete_tail: false,
        },
    )
    .unwrap()
}

fn record(base: u64, transaction: u64, value: &[u8], entry_count: u64) -> WalRecord {
    let mut changes = ChangeSet::new();
    changes.insert(b"key", Some(value.to_vec()));
    WalRecord {
        kind: WalRecordKind::Transaction,
        base_transaction_id: TransactionId::from_raw(base),
        transaction_id: TransactionId::from_raw(transaction),
        entry_count,
        changes,
    }
}
