use std::fs::File;
use std::time::{Duration, Instant};

use super::checksum::{crc32c, crc32c_combine};
use super::fault_injection;
use super::positioned_io::{read_exact_at, write_all_at};
use crate::allocation::{clone_bytes, zeroed_bytes};
use crate::domain::{ChangeSet, MAX_KEY_BYTES, MAX_VALUE_BYTES};
use crate::{Error, Result, TransactionId};

mod file;
mod group;
mod handoff;
mod snapshot;

pub(crate) use file::{file_len, is_current, open, path};
pub(crate) use handoff::WalHandoff;
pub(crate) use snapshot::replace_with_snapshot_streamed;

const TRANSACTION_MAGIC: [u8; 8] = *b"WSLDBWAL";
const SNAPSHOT_MAGIC: [u8; 8] = *b"WSLDBSNP";
const FOOTER_MAGIC: [u8; 4] = *b"WEND";
const HEADER_BYTES: usize = 52;
const FOOTER_BYTES: usize = 16;
const MIN_RECORD_BYTES: usize = HEADER_BYTES + FOOTER_BYTES;
const MAX_RECORD_BYTES: u64 = 1024 * 1024 * 1024;
const TOMBSTONE_LENGTH: u64 = u64::MAX;
const MIN_MUTATION_BYTES: usize = 2 + 2 + 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PhysicalRecordKind {
    Transaction,
    Snapshot,
    Group,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WalRecordKind {
    Transaction,
    Snapshot,
}

pub(crate) struct WalRecord {
    pub(crate) kind: WalRecordKind,
    pub(crate) base_transaction_id: TransactionId,
    pub(crate) transaction_id: TransactionId,
    pub(crate) entry_count: u64,
    pub(crate) changes: ChangeSet,
}

#[cfg(test)]
pub(crate) struct WalLoad {
    pub(crate) records: Vec<WalRecord>,
    pub(crate) valid_bytes: u64,
    pub(crate) has_incomplete_tail: bool,
}

pub(crate) struct WalScan {
    pub(crate) valid_bytes: u64,
    pub(crate) has_incomplete_tail: bool,
}

pub(crate) struct WalAppend {
    pub(crate) bytes: u64,
    pub(crate) end: u64,
    pub(crate) boundary: Duration,
    pub(crate) encode: Duration,
    pub(crate) write: Duration,
    pub(crate) sync: Duration,
}

pub(crate) struct SnapshotReplacement {
    pub(crate) file: File,
    pub(crate) bytes: u64,
    pub(crate) sync: Duration,
}

pub(crate) fn append(
    file: &File,
    expected_end: u64,
    base_transaction_id: TransactionId,
    transaction_id: TransactionId,
    entry_count: u64,
    changes: &ChangeSet,
) -> Result<WalAppend> {
    let encode_started = Instant::now();
    let bytes = encode(base_transaction_id, transaction_id, entry_count, changes)?;
    let encode = encode_started.elapsed();
    append_bytes(file, expected_end, &bytes, encode)
}

pub(crate) fn append_group(
    file: &File,
    expected_end: u64,
    records: &[WalRecord],
) -> Result<WalAppend> {
    if records.is_empty() {
        return Err(Error::InternalInvariant("write-ahead group is empty"));
    }
    let encode_started = Instant::now();
    let bytes = group::encode(records)?;
    let encode = encode_started.elapsed();
    append_bytes(file, expected_end, &bytes, encode)
}

fn append_bytes(
    file: &File,
    expected_end: u64,
    bytes: &[u8],
    encode: Duration,
) -> Result<WalAppend> {
    let boundary_started = Instant::now();
    if file.metadata()?.len() != expected_end {
        return Err(Error::WriteConflict);
    }
    let appended = u64::try_from(bytes.len()).map_err(|_| Error::FileTooLarge)?;
    let end = expected_end
        .checked_add(appended)
        .ok_or(Error::FileTooLarge)?;
    let boundary = boundary_started.elapsed();
    let write_started = Instant::now();
    if let Err(error) = write_all_at(file, bytes, expected_end) {
        if let Err(rollback) = truncate(file, expected_end) {
            return match rollback {
                Error::Io(rollback) => Err(Error::CommitOutcomeUnknown(rollback)),
                other => Err(other),
            };
        }
        return Err(error.into());
    }
    let write = write_started.elapsed();
    super::crash_injection::hit("write_buffer_write");
    fault_injection::before("write_buffer_sync").map_err(Error::CommitOutcomeUnknown)?;
    let sync_started = Instant::now();
    file.sync_all().map_err(Error::CommitOutcomeUnknown)?;
    let sync = sync_started.elapsed();
    Ok(WalAppend {
        bytes: appended,
        end,
        boundary,
        encode,
        write,
        sync,
    })
}

#[cfg(test)]
pub(crate) fn load(file: &File) -> Result<WalLoad> {
    let mut records = Vec::new();
    let scan = replay(file, |record| {
        records.push(record);
        Ok(())
    })?;
    Ok(WalLoad {
        records,
        valid_bytes: scan.valid_bytes,
        has_incomplete_tail: scan.has_incomplete_tail,
    })
}

pub(crate) fn replay(file: &File, consume: impl FnMut(WalRecord) -> Result<()>) -> Result<WalScan> {
    replay_from(file, 0, None, consume)
}

pub(crate) fn replay_after(
    file: &File,
    offset: u64,
    expected_base: TransactionId,
    consume: impl FnMut(WalRecord) -> Result<()>,
) -> Result<WalScan> {
    replay_from(file, offset, Some(expected_base), consume)
}

fn replay_from(
    file: &File,
    mut offset: u64,
    mut expected_base: Option<TransactionId>,
    mut consume: impl FnMut(WalRecord) -> Result<()>,
) -> Result<WalScan> {
    let file_len = file.metadata()?.len();
    if offset > file_len {
        return Err(Error::Corrupt(
            "write-ahead replay offset exceeds the file length".into(),
        ));
    }

    while offset < file_len {
        let remaining = file_len.checked_sub(offset).ok_or(Error::FileTooLarge)?;
        if remaining < HEADER_BYTES as u64 {
            return Ok(WalScan {
                valid_bytes: offset,
                has_incomplete_tail: true,
            });
        }
        let mut header = [0_u8; HEADER_BYTES];
        read_exact_at(file, &mut header, offset)?;
        let physical_kind = validate_header(&header)?;
        let record_len = read_u64(&header[8..16])?;
        if !(MIN_RECORD_BYTES as u64..=MAX_RECORD_BYTES).contains(&record_len) {
            return Err(Error::Corrupt(
                "write-ahead record length is invalid".into(),
            ));
        }
        if record_len > remaining {
            return Ok(WalScan {
                valid_bytes: offset,
                has_incomplete_tail: true,
            });
        }
        let record_len = usize::try_from(record_len)
            .map_err(|_| Error::Corrupt("write-ahead record is too large".into()))?;
        let mut bytes = allocate_record_buffer(record_len)?;
        read_exact_at(file, &mut bytes, offset)?;
        let records = match physical_kind {
            PhysicalRecordKind::Group => group::decode(&bytes)?,
            PhysicalRecordKind::Transaction | PhysicalRecordKind::Snapshot => {
                vec![decode(&bytes)?]
            }
        };
        if physical_kind == PhysicalRecordKind::Snapshot && offset != 0 {
            return Err(Error::Corrupt(
                "write-ahead snapshot is not the first record".into(),
            ));
        }
        for record in records {
            if expected_base.is_some_and(|expected| record.base_transaction_id != expected) {
                return Err(Error::Corrupt(
                    "write-ahead transaction chain is discontinuous".into(),
                ));
            }
            expected_base = Some(record.transaction_id);
            consume(record)?;
        }
        offset = offset
            .checked_add(u64::try_from(record_len).map_err(|_| Error::FileTooLarge)?)
            .ok_or(Error::FileTooLarge)?;
    }

    Ok(WalScan {
        valid_bytes: offset,
        has_incomplete_tail: false,
    })
}

fn allocate_record_buffer(record_len: usize) -> Result<Vec<u8>> {
    zeroed_bytes(record_len)
}

pub(crate) fn truncate(file: &File, len: u64) -> Result<()> {
    file.set_len(len)?;
    super::crash_injection::hit("wal_truncate");
    fault_injection::before("wal_truncate_sync")?;
    file.sync_all()?;
    Ok(())
}

fn encode(
    base_transaction_id: TransactionId,
    transaction_id: TransactionId,
    entry_count: u64,
    changes: &ChangeSet,
) -> Result<Vec<u8>> {
    encode_record(
        WalRecordKind::Transaction,
        base_transaction_id,
        transaction_id,
        entry_count,
        changes,
    )
}

fn encode_record(
    kind: WalRecordKind,
    base_transaction_id: TransactionId,
    transaction_id: TransactionId,
    entry_count: u64,
    changes: &ChangeSet,
) -> Result<Vec<u8>> {
    validate_sequence(kind, base_transaction_id, transaction_id)
        .map_err(|_| Error::InternalInvariant("write-ahead transaction sequence"))?;
    let mutation_count = u32::try_from(changes.len()).map_err(|_| Error::FileTooLarge)?;
    let payload_len = encoded_payload_len(changes)?;
    let magic = match kind {
        WalRecordKind::Transaction => TRANSACTION_MAGIC,
        WalRecordKind::Snapshot => SNAPSHOT_MAGIC,
    };
    encode_frame(
        magic,
        base_transaction_id,
        transaction_id,
        entry_count,
        mutation_count,
        payload_len,
        |bytes| encode_changes(bytes, changes),
    )
}

fn encode_frame(
    magic: [u8; 8],
    base_transaction_id: TransactionId,
    transaction_id: TransactionId,
    entry_count: u64,
    item_count: u32,
    payload_len: usize,
    encode_payload: impl FnOnce(&mut Vec<u8>) -> Result<()>,
) -> Result<Vec<u8>> {
    let total_len = HEADER_BYTES
        .checked_add(payload_len)
        .and_then(|len| len.checked_add(FOOTER_BYTES))
        .ok_or(Error::FileTooLarge)?;
    let total_len_u64 = u64::try_from(total_len).map_err(|_| Error::FileTooLarge)?;
    if total_len_u64 > MAX_RECORD_BYTES {
        return Err(Error::FileTooLarge);
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total_len)
        .map_err(|_| Error::FileTooLarge)?;
    bytes.resize(HEADER_BYTES, 0);
    encode_payload(&mut bytes)?;
    if bytes.len() != HEADER_BYTES + payload_len {
        return Err(Error::InternalInvariant(
            "write-ahead payload length changed during encoding",
        ));
    }
    bytes[..8].copy_from_slice(&magic);
    bytes[8..16].copy_from_slice(&total_len_u64.to_le_bytes());
    bytes[16..24].copy_from_slice(&base_transaction_id.value().to_le_bytes());
    bytes[24..32].copy_from_slice(&transaction_id.value().to_le_bytes());
    bytes[32..40].copy_from_slice(&entry_count.to_le_bytes());
    bytes[40..44].copy_from_slice(&item_count.to_le_bytes());
    let payload_crc = crc32c(&bytes[HEADER_BYTES..]);
    bytes[44..48].copy_from_slice(&payload_crc.to_le_bytes());
    let header_crc = crc32c(&bytes[..48]);
    bytes[48..52].copy_from_slice(&header_crc.to_le_bytes());
    let record_crc = crc32c_combine(crc32c(&bytes[..HEADER_BYTES]), payload_crc, payload_len);
    bytes.extend_from_slice(&record_crc.to_le_bytes());
    bytes.extend_from_slice(&FOOTER_MAGIC);
    bytes.extend_from_slice(&total_len_u64.to_le_bytes());
    Ok(bytes)
}

fn encode_changes(bytes: &mut Vec<u8>, changes: &ChangeSet) -> Result<()> {
    let mut previous_key: &[u8] = &[];
    for (key, value) in changes.sorted_iter() {
        let prefix = common_prefix_len(previous_key, key);
        let suffix = &key[prefix..];
        push_u16(bytes, prefix)?;
        push_u16(bytes, suffix.len())?;
        bytes.extend_from_slice(suffix);
        match value {
            Some(value) => {
                push_u64(bytes, value.len())?;
                bytes.extend_from_slice(value);
            }
            None => bytes.extend_from_slice(&TOMBSTONE_LENGTH.to_le_bytes()),
        }
        previous_key = key;
    }
    Ok(())
}

fn encoded_payload_len(changes: &ChangeSet) -> Result<usize> {
    let mut bytes = 0_usize;
    let mut previous_key: &[u8] = &[];
    for (key, value) in changes.sorted_iter() {
        let suffix_len = key
            .len()
            .saturating_sub(common_prefix_len(previous_key, key));
        let value_len = value.map_or(0, <[u8]>::len);
        bytes = bytes
            .checked_add(12)
            .and_then(|bytes| bytes.checked_add(suffix_len))
            .and_then(|bytes| bytes.checked_add(value_len))
            .ok_or(Error::FileTooLarge)?;
        previous_key = key;
    }
    Ok(bytes)
}

fn common_prefix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| left.len().min(right.len()))
}

fn validate_header(header: &[u8; HEADER_BYTES]) -> Result<PhysicalRecordKind> {
    let kind = if header[..8] == TRANSACTION_MAGIC {
        PhysicalRecordKind::Transaction
    } else if header[..8] == SNAPSHOT_MAGIC {
        PhysicalRecordKind::Snapshot
    } else if header[..8] == group::MAGIC {
        PhysicalRecordKind::Group
    } else {
        return Err(Error::Corrupt(
            "write-ahead record magic is not recognized".into(),
        ));
    };
    if read_u32(&header[48..52])? != crc32c(&header[..48]) {
        return Err(Error::Corrupt(
            "write-ahead record header checksum failed".into(),
        ));
    }
    Ok(kind)
}

struct DecodedEnvelope<'a> {
    kind: PhysicalRecordKind,
    base_transaction_id: TransactionId,
    transaction_id: TransactionId,
    entry_count: u64,
    item_count: u32,
    payload: &'a [u8],
}

fn decode_envelope(bytes: &[u8]) -> Result<DecodedEnvelope<'_>> {
    if bytes.len() < MIN_RECORD_BYTES {
        return Err(Error::Corrupt("write-ahead record is truncated".into()));
    }
    let header: &[u8; HEADER_BYTES] = bytes[..HEADER_BYTES]
        .try_into()
        .map_err(|_| Error::InternalInvariant("write-ahead header width"))?;
    let kind = validate_header(header)?;
    let total_len = usize::try_from(read_u64(&bytes[8..16])?)
        .map_err(|_| Error::Corrupt("write-ahead record length does not fit in memory".into()))?;
    if total_len != bytes.len() || total_len < MIN_RECORD_BYTES {
        return Err(Error::Corrupt(
            "write-ahead record length is invalid".into(),
        ));
    }
    let footer = total_len - FOOTER_BYTES;
    if bytes[footer + 4..footer + 8] != FOOTER_MAGIC
        || usize::try_from(read_u64(&bytes[footer + 8..total_len])?).ok() != Some(total_len)
        || read_u32(&bytes[footer..footer + 4])? != crc32c(&bytes[..footer])
    {
        return Err(Error::Corrupt("write-ahead record checksum failed".into()));
    }
    let payload = &bytes[HEADER_BYTES..footer];
    if read_u32(&bytes[44..48])? != crc32c(payload) {
        return Err(Error::Corrupt("write-ahead payload checksum failed".into()));
    }

    Ok(DecodedEnvelope {
        kind,
        base_transaction_id: TransactionId::from_raw(read_u64(&bytes[16..24])?),
        transaction_id: TransactionId::from_raw(read_u64(&bytes[24..32])?),
        entry_count: read_u64(&bytes[32..40])?,
        item_count: read_u32(&bytes[40..44])?,
        payload,
    })
}

fn decode(bytes: &[u8]) -> Result<WalRecord> {
    let envelope = decode_envelope(bytes)?;
    let kind = match envelope.kind {
        PhysicalRecordKind::Transaction => WalRecordKind::Transaction,
        PhysicalRecordKind::Snapshot => WalRecordKind::Snapshot,
        PhysicalRecordKind::Group => {
            return Err(Error::Corrupt(
                "write-ahead group requires grouped decoding".into(),
            ));
        }
    };

    if validate_sequence(kind, envelope.base_transaction_id, envelope.transaction_id).is_err() {
        return Err(Error::Corrupt(
            "write-ahead transaction sequence is invalid".into(),
        ));
    }
    let mutation_count = usize::try_from(envelope.item_count)
        .map_err(|_| Error::Corrupt("write-ahead mutation count does not fit in memory".into()))?;
    if mutation_count > envelope.payload.len() / MIN_MUTATION_BYTES {
        return Err(Error::Corrupt(
            "write-ahead mutation count exceeds its payload".into(),
        ));
    }
    let mut input = Input::new(envelope.payload);
    let changes = decode_changes(&mut input, mutation_count)?;
    if !input.is_empty() {
        return Err(Error::Corrupt(
            "write-ahead payload has trailing bytes".into(),
        ));
    }
    Ok(WalRecord {
        kind,
        base_transaction_id: envelope.base_transaction_id,
        transaction_id: envelope.transaction_id,
        entry_count: envelope.entry_count,
        changes,
    })
}

fn decode_changes(input: &mut Input<'_>, mutation_count: usize) -> Result<ChangeSet> {
    let mut changes = ChangeSet::new();
    let mut previous_key: Option<Vec<u8>> = None;
    for _ in 0..mutation_count {
        let prefix_len = usize::from(input.read_u16()?);
        let suffix_len = usize::from(input.read_u16()?);
        if prefix_len > previous_key.as_ref().map_or(0, Vec::len) {
            return Err(Error::Corrupt(
                "write-ahead key prefix length is invalid".into(),
            ));
        }
        let key_len = prefix_len
            .checked_add(suffix_len)
            .ok_or_else(|| Error::Corrupt("write-ahead key length overflow".into()))?;
        if key_len == 0 || key_len > MAX_KEY_BYTES {
            return Err(Error::Corrupt("write-ahead key length is invalid".into()));
        }
        let mut key = Vec::new();
        key.try_reserve_exact(key_len)
            .map_err(|_| Error::Corrupt("write-ahead key is too large".into()))?;
        if let Some(previous) = &previous_key {
            key.extend_from_slice(&previous[..prefix_len]);
        }
        key.extend_from_slice(input.take(suffix_len)?);
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous >= &key)
        {
            return Err(Error::Corrupt(
                "write-ahead keys are not strictly ordered".into(),
            ));
        }
        let value_len = input.read_u64()?;
        let value = if value_len == TOMBSTONE_LENGTH {
            None
        } else {
            let value_len = usize::try_from(value_len).map_err(|_| {
                Error::Corrupt("write-ahead value length does not fit in memory".into())
            })?;
            if value_len > MAX_VALUE_BYTES {
                return Err(Error::Corrupt("write-ahead value is too large".into()));
            }
            Some(clone_bytes(input.take(value_len)?)?)
        };
        changes.insert(&key, value);
        previous_key = Some(key);
    }
    Ok(changes)
}

fn validate_sequence(
    kind: WalRecordKind,
    base_transaction_id: TransactionId,
    transaction_id: TransactionId,
) -> Result<()> {
    let valid = match kind {
        WalRecordKind::Transaction => transaction_id == base_transaction_id.next()?,
        WalRecordKind::Snapshot => transaction_id > base_transaction_id,
    };
    if !valid {
        return Err(Error::InternalInvariant("write-ahead transaction sequence"));
    }
    Ok(())
}

fn push_u16(bytes: &mut Vec<u8>, value: usize) -> Result<()> {
    bytes.extend_from_slice(
        &u16::try_from(value)
            .map_err(|_| Error::FileTooLarge)?
            .to_le_bytes(),
    );
    Ok(())
}

fn push_u64(bytes: &mut Vec<u8>, value: usize) -> Result<()> {
    bytes.extend_from_slice(
        &u64::try_from(value)
            .map_err(|_| Error::FileTooLarge)?
            .to_le_bytes(),
    );
    Ok(())
}

fn read_u32(bytes: &[u8]) -> Result<u32> {
    bytes
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| Error::Corrupt("write-ahead u32 field is truncated".into()))
}

fn read_u64(bytes: &[u8]) -> Result<u64> {
    bytes
        .try_into()
        .map(u64::from_le_bytes)
        .map_err(|_| Error::Corrupt("write-ahead u64 field is truncated".into()))
}

struct Input<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Input<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(len)
            .ok_or_else(|| Error::Corrupt("write-ahead field length overflow".into()))?;
        let result = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| Error::Corrupt("write-ahead payload is truncated".into()))?;
        self.position = end;
        Ok(result)
    }

    fn read_u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(
            |_| Error::InternalInvariant("write-ahead u16 width"),
        )?))
    }

    fn read_u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(
            |_| Error::InternalInvariant("write-ahead u64 width"),
        )?))
    }

    const fn is_empty(&self) -> bool {
        self.position == self.bytes.len()
    }

    const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
}

#[cfg(test)]
mod tests;
