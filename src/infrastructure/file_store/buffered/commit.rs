use std::time::Instant;

use super::{BufferedWrites, DurableBufferCommit};
use crate::application::CommittedStorage;
use crate::domain::{ChangeSet, Head};
use crate::infrastructure::crash_injection;
use crate::infrastructure::file_context::ReadContext;
use crate::infrastructure::file_store::FileStorageEngine;
use crate::infrastructure::{tree, wal};
use crate::{Error, Result};

impl FileStorageEngine {
    pub(in crate::infrastructure::file_store) fn commit_buffered(
        &self,
        buffer: &BufferedWrites,
        base: Head,
        changes: ChangeSet,
        commit_started: Instant,
    ) -> Result<CommittedStorage> {
        let point_index_initialization = buffer.point_index.lock_if_empty()?;
        let state = Self::buffer_state(buffer)?;
        let physical_checkpoint = buffer.physical_checkpoint()?;
        let opened_file =
            self.open_main_for_buffered_commit(buffer, &state, physical_checkpoint)?;
        let file = opened_file.as_ref().unwrap_or(&self.file);
        if state.head() != base || !self.buffer_file_matches(buffer, &state)? {
            return Err(Error::WriteConflict);
        }
        let filter_may_grow = state.key_filter_needs_growth(changes.len())?;
        let transaction_id = base.transaction_id().next()?;
        let mut entry_count = base.entry_count();
        let mut new_buffer_keys = 0_u64;
        let mut ordered_presence = if changes.len() == 1 {
            None
        } else {
            let mut presence = Vec::new();
            presence
                .try_reserve_exact(changes.len())
                .map_err(|_| Error::FileTooLarge)?;
            Some(presence)
        };
        let mut singleton_presence = None;
        for (key, value) in changes.sorted_iter() {
            let buffered = state.visible_presence(base, key)?;
            if filter_may_grow && buffered.is_none() {
                new_buffer_keys = new_buffer_keys.checked_add(1).ok_or(Error::FileTooLarge)?;
            }
            match &mut ordered_presence {
                Some(ordered) => ordered.push(buffered.is_some()),
                None => singleton_presence = Some(buffered.is_some()),
            }
            let base_presence = if buffered.is_none() {
                buffer.base_visible_presence(key)?
            } else {
                None
            };
            let existed = match (buffered, base_presence) {
                (Some(present), _) | (None, Some(present)) => present,
                (None, None) if !buffer.checkpoint_may_contain(key)? => false,
                (None, None) => tree::get(
                    &ReadContext::new(file, &self.page_cache, physical_checkpoint),
                    physical_checkpoint.root(),
                    key,
                )?
                .is_some(),
            };
            match (existed, value.is_some()) {
                (false, true) => {
                    entry_count = entry_count.checked_add(1).ok_or(Error::FileTooLarge)?;
                }
                (true, false) => {
                    entry_count = entry_count
                        .checked_sub(1)
                        .ok_or(Error::InternalInvariant("entry count underflow"))?;
                }
                _ => {}
            }
        }
        let wal_end = state.wal_bytes();
        let logical_mutation_bytes = changes.logical_bytes()?;
        let mut point_updates = Vec::new();
        if let Some(point_index) = buffer.point_index.get() {
            point_updates
                .try_reserve_exact(changes.len())
                .map_err(|_| Error::FileTooLarge)?;
            if let Some(present) = singleton_presence {
                let (key, _) = changes
                    .sorted_iter()
                    .next()
                    .ok_or(Error::InternalInvariant("singleton change is empty"))?;
                point_index.preflight_one(key, present)?;
            } else if let Some(ordered) = &ordered_presence {
                point_index.preflight_known(&changes, ordered)?;
            }
        }
        drop(state);
        // If the index was absent, its initialization guard remains held until
        // publication. A first reader therefore builds from the new head
        // instead of requiring a post-sync catch-up allocation.
        let mut state = buffer
            .state
            .write()
            .map_err(|_| Error::LockPoisoned("write buffer"))?;
        if filter_may_grow {
            state.prepare_key_filter(new_buffer_keys)?;
        }
        state.prepare_apply(std::iter::once(&changes))?;
        drop(state);

        let preflight = commit_started.elapsed();
        let sync_started = Instant::now();
        let appended = match wal::append(
            &buffer.file,
            wal_end,
            base.transaction_id(),
            transaction_id,
            entry_count,
            &changes,
        ) {
            Ok(appended) => appended,
            Err(error) => {
                let mut state = match buffer.state.write() {
                    Ok(state) => state,
                    Err(poisoned) => poisoned.into_inner(),
                };
                state.discard_prepared();
                return Err(error);
            }
        };
        let publication_started = Instant::now();
        self.metrics
            .record_wal_append(appended.encode, appended.write, appended.sync);
        let wal_sync = sync_started.elapsed();
        crash_injection::hit("write_buffer_sync");
        let new_wal_bytes = appended.end;
        // Only readers could run since preflight; read guards do not poison.
        // Recovering this guard keeps durable commit publication infallible.
        let mut state = match buffer.state.write() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let point_index = buffer.point_index.get();
        state.apply_durable(
            wal::WalRecord {
                kind: wal::WalRecordKind::Transaction,
                base_transaction_id: base.transaction_id(),
                transaction_id,
                entry_count,
                changes,
            },
            point_index.map(|_| &mut point_updates),
        );
        state.set_wal_bytes(new_wal_bytes);
        let logical_head = state.head();
        if let Some(point_index) = point_index {
            point_index.apply_durable(point_updates);
        }
        drop(state);
        drop(point_index_initialization);

        let writes = crate::WriteStats {
            logical_mutation_bytes,
            data_page_bytes: 0,
            leaf_page_bytes: 0,
            branch_page_bytes: 0,
            overflow_page_bytes: 0,
            write_buffer_bytes: appended.bytes,
            metadata_bytes: 0,
        };
        Ok(self.finish_buffered_commit(
            buffer,
            file,
            DurableBufferCommit {
                head: logical_head,
                wal_bytes: new_wal_bytes,
                writes,
                sync: wal_sync,
                preflight: preflight.saturating_add(appended.boundary),
                publication_started,
            },
            commit_started,
        ))
    }
}
