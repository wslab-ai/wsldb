use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use super::{BufferedWrites, DurableBufferCommit};
use crate::application::CommittedStorage;
use crate::domain::{ChangeSet, Head};
use crate::infrastructure::crash_injection;
use crate::infrastructure::file_context::ReadContext;
use crate::infrastructure::file_store::FileStorageEngine;
use crate::infrastructure::{tree, wal};
use crate::{Error, Result};

pub(in crate::infrastructure::file_store) fn commit(
    engine: &FileStorageEngine,
    buffer: &BufferedWrites,
    base: Head,
    changes: Vec<ChangeSet>,
    commit_started: Instant,
) -> Result<CommittedStorage> {
    if changes.is_empty() {
        return Err(Error::InternalInvariant("buffered commit group is empty"));
    }
    let point_index_initialization = buffer.point_index.lock_if_empty()?;
    let state = FileStorageEngine::buffer_state(buffer)?;
    let physical_checkpoint = buffer.physical_checkpoint()?;
    let opened_file = engine.open_main_for_buffered_commit(buffer, &state, physical_checkpoint)?;
    let file = opened_file.as_ref().unwrap_or(&engine.file);
    if state.head() != base || !engine.buffer_file_matches(buffer, &state)? {
        return Err(Error::WriteConflict);
    }
    let mutation_upper_bound = changes.iter().try_fold(0_usize, |total, changes| {
        total.checked_add(changes.len()).ok_or(Error::FileTooLarge)
    })?;
    let filter_may_grow = state.key_filter_needs_growth(mutation_upper_bound)?;

    let wal_end = state.wal_bytes();
    let mut transaction_id = base.transaction_id();
    let mut entry_count = base.entry_count();
    let mut overlay = HashMap::<Arc<[u8]>, bool>::new();
    overlay
        .try_reserve(mutation_upper_bound)
        .map_err(|_| Error::FileTooLarge)?;
    let mut records = Vec::new();
    records
        .try_reserve_exact(changes.len())
        .map_err(|_| Error::FileTooLarge)?;
    let mut logical_mutation_bytes = 0_u64;
    let mut new_buffer_keys = 0_u64;
    let preflight_point_index = buffer.point_index.get();
    for changes in changes {
        logical_mutation_bytes = logical_mutation_bytes
            .checked_add(changes.logical_bytes()?)
            .ok_or(Error::FileTooLarge)?;
        let mut ordered_presence = Vec::new();
        ordered_presence
            .try_reserve_exact(changes.len())
            .map_err(|_| Error::FileTooLarge)?;
        for (key, value) in changes.shared_sorted_iter() {
            let buffered = state.visible_presence(base, key)?;
            ordered_presence.push(buffered.is_some());
            let base_presence = if buffered.is_none() {
                buffer.base_visible_presence(key)?
            } else {
                None
            };
            let overlay_presence = overlay.get(key.as_ref()).copied();
            if filter_may_grow && overlay_presence.is_none() && buffered.is_none() {
                new_buffer_keys = new_buffer_keys.checked_add(1).ok_or(Error::FileTooLarge)?;
            }
            let existed = match overlay_presence {
                Some(existed) => existed,
                None => match (buffered, base_presence) {
                    (Some(present), _) | (None, Some(present)) => present,
                    (None, None) if !buffer.checkpoint_may_contain(key)? => false,
                    (None, None) => tree::get(
                        &ReadContext::new(file, &engine.page_cache, physical_checkpoint),
                        physical_checkpoint.root(),
                        key,
                    )?
                    .is_some(),
                },
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
            overlay.insert(Arc::clone(key), value.is_some());
        }
        if let Some(point_index) = preflight_point_index {
            point_index.preflight_known(&changes, &ordered_presence)?;
        }
        let next = transaction_id.next()?;
        records.push(wal::WalRecord {
            kind: wal::WalRecordKind::Transaction,
            base_transaction_id: transaction_id,
            transaction_id: next,
            entry_count,
            changes,
        });
        transaction_id = next;
    }
    let mut point_updates = Vec::new();
    if preflight_point_index.is_some() {
        point_updates
            .try_reserve_exact(mutation_upper_bound)
            .map_err(|_| Error::FileTooLarge)?;
    }
    drop(state);

    // Poisoning is the last ordinary failure possible before durable append.
    let mut state = buffer
        .state
        .write()
        .map_err(|_| Error::LockPoisoned("write buffer"))?;
    if filter_may_grow {
        state.prepare_key_filter(new_buffer_keys)?;
    }
    state.prepare_apply(records.iter().map(|record| &record.changes))?;
    drop(state);
    let preflight = commit_started.elapsed();
    let sync_started = Instant::now();
    let appended = match wal::append_group(&buffer.file, wal_end, &records) {
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
    engine
        .metrics
        .record_wal_append(appended.encode, appended.write, appended.sync);
    let wal_sync = sync_started.elapsed();
    crash_injection::hit("write_buffer_sync");

    let mut state = match buffer.state.write() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    let point_index = buffer.point_index.get();
    for record in records {
        state.apply_durable(record, point_index.map(|_| &mut point_updates));
    }
    state.set_wal_bytes(appended.end);
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
    Ok(engine.finish_buffered_commit(
        buffer,
        file,
        DurableBufferCommit {
            head: logical_head,
            wal_bytes: appended.end,
            writes,
            sync: wal_sync,
            preflight: preflight.saturating_add(appended.boundary),
            publication_started,
        },
        commit_started,
    ))
}
