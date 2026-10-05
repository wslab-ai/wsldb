use std::sync::Arc;

use super::{PointUpdate, WriteBuffer};
use crate::infrastructure::version_arena::append_version;
use crate::infrastructure::wal::{WalRecord, WalRecordKind};
use crate::{Error, Result};

impl WriteBuffer {
    #[cfg(test)]
    pub(crate) fn apply(&mut self, record: WalRecord) -> Result<Vec<PointUpdate>> {
        self.apply_record(record, true)
    }

    pub(crate) fn apply_durable(
        &mut self,
        record: WalRecord,
        point_updates: Option<&mut Vec<PointUpdate>>,
    ) {
        self.apply_record_unchecked(record, point_updates);
    }

    pub(crate) fn replay(&mut self, record: WalRecord) -> Result<()> {
        self.apply_record(record, false).map(|_| ())
    }

    pub(super) fn apply_record(
        &mut self,
        record: WalRecord,
        collect_point_updates: bool,
    ) -> Result<Vec<PointUpdate>> {
        let valid_sequence = match record.kind {
            WalRecordKind::Transaction => {
                record.base_transaction_id == self.head.transaction_id()
                    && record.transaction_id == self.head.transaction_id().next()?
            }
            WalRecordKind::Snapshot => {
                self.head == self.checkpoint
                    && self.versioned_keys == 0
                    && record.base_transaction_id == self.checkpoint.transaction_id()
                    && record.transaction_id > record.base_transaction_id
            }
        };
        if !valid_sequence {
            return Err(Error::Corrupt(
                "write buffer transaction chain is discontinuous".into(),
            ));
        }
        if self.key_filter_needs_growth(record.changes.len())? {
            let additional_keys = record
                .changes
                .sorted_iter()
                .filter(|(key, _)| !self.contains_key(key))
                .count();
            self.prepare_key_filter(
                u64::try_from(additional_keys).map_err(|_| Error::FileTooLarge)?,
            )?;
        }
        let mut point_updates = Vec::new();
        if collect_point_updates {
            point_updates
                .try_reserve_exact(record.changes.len())
                .map_err(|_| Error::FileTooLarge)?;
        }
        self.prepare_apply(std::iter::once(&record.changes))?;
        self.apply_record_unchecked(record, collect_point_updates.then_some(&mut point_updates));
        Ok(point_updates)
    }

    fn apply_record_unchecked(
        &mut self,
        record: WalRecord,
        mut point_updates: Option<&mut Vec<PointUpdate>>,
    ) {
        let transaction_id = record.transaction_id;
        self.consolidated = record.kind == WalRecordKind::Snapshot;
        let arena = Arc::clone(&self.arena);
        let mut arena = arena.write_durable();
        for (key, value) in record.changes.into_sorted_iter() {
            // `prepare_apply` creates every missing ordered-map node before
            // WAL synchronization. The `or_insert` is a defensive fallback;
            // on the valid path this only finds an occupied node.
            let slot = self.versions.entry(Arc::clone(&key)).or_insert(None);
            let previous = *slot;
            if previous.is_none() {
                if let Some(filter) = &mut self.key_filter {
                    filter.insert(&key);
                }
                self.versioned_keys += 1;
            }
            let version = append_version(&mut arena, transaction_id, value, previous);
            *slot = Some(version);
            if let Some(updates) = &mut point_updates {
                updates.push(PointUpdate { key, version });
            }
        }
        self.head = crate::domain::Head::new(
            transaction_id,
            self.checkpoint.root(),
            self.checkpoint.last_page(),
            record.entry_count,
        );
    }
}
