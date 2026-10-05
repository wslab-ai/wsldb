use super::{
    DecodedEnvelope, Input, MAX_RECORD_BYTES, PhysicalRecordKind, WalRecord, WalRecordKind,
    common_prefix_len, decode_envelope, encode_frame, validate_sequence,
};
use crate::allocation::clone_bytes;
use crate::domain::{ChangeSet, MAX_KEY_BYTES, MAX_VALUE_BYTES};
use crate::{Error, MAX_GROUP_TRANSACTIONS, Result};

pub(super) const MAGIC: [u8; 8] = *b"WSLDBGRP";

pub(super) fn encode(records: &[WalRecord]) -> Result<Vec<u8>> {
    if records.len() > MAX_GROUP_TRANSACTIONS {
        return Err(Error::FileTooLarge);
    }
    let first = records
        .first()
        .ok_or(Error::InternalInvariant("write-ahead group is empty"))?;
    let last = records
        .last()
        .ok_or(Error::InternalInvariant("write-ahead group is empty"))?;
    let transaction_count = u32::try_from(records.len()).map_err(|_| Error::FileTooLarge)?;
    let mut expected_base = first.base_transaction_id;
    for record in records {
        if record.kind != WalRecordKind::Transaction {
            return Err(Error::InternalInvariant(
                "write-ahead group contains a snapshot",
            ));
        }
        if record.base_transaction_id != expected_base {
            return Err(Error::InternalInvariant(
                "write-ahead group is discontinuous",
            ));
        }
        validate_sequence(
            record.kind,
            record.base_transaction_id,
            record.transaction_id,
        )
        .map_err(|_| Error::InternalInvariant("write-ahead transaction sequence"))?;
        expected_base = record.transaction_id;
    }

    let payload_len = encoded_payload_len(records)?;
    encode_frame(
        MAGIC,
        first.base_transaction_id,
        last.transaction_id,
        last.entry_count,
        transaction_count,
        payload_len,
        |bytes| encode_payload(bytes, records),
    )
}

pub(super) fn decode(bytes: &[u8]) -> Result<Vec<WalRecord>> {
    let envelope = decode_envelope(bytes)?;
    if envelope.kind != PhysicalRecordKind::Group {
        return Err(Error::Corrupt("write-ahead record is not a group".into()));
    }
    decode_payload(envelope)
}

fn decode_payload(envelope: DecodedEnvelope<'_>) -> Result<Vec<WalRecord>> {
    let transaction_count = usize::try_from(envelope.item_count)
        .map_err(|_| Error::Corrupt("write-ahead group count does not fit in memory".into()))?;
    if transaction_count == 0 {
        return Err(Error::Corrupt("write-ahead group is empty".into()));
    }
    if transaction_count > MAX_GROUP_TRANSACTIONS {
        return Err(Error::Corrupt(
            "write-ahead group exceeds the transaction limit".into(),
        ));
    }
    if transaction_count > envelope.payload.len() / 2 {
        return Err(Error::Corrupt(
            "write-ahead group count exceeds its payload".into(),
        ));
    }
    let count_u64 = u64::try_from(transaction_count)
        .map_err(|_| Error::Corrupt("write-ahead group count does not fit in u64".into()))?;
    let expected_final = envelope
        .base_transaction_id
        .value()
        .checked_add(count_u64)
        .ok_or_else(|| Error::Corrupt("write-ahead group transaction ID overflow".into()))?;
    if envelope.transaction_id.value() != expected_final {
        return Err(Error::Corrupt(
            "write-ahead group final transaction ID is invalid".into(),
        ));
    }

    let mut records = Vec::new();
    records
        .try_reserve_exact(transaction_count)
        .map_err(|_| Error::Corrupt("write-ahead group is too large".into()))?;
    let mut input = Input::new(envelope.payload);
    let mut previous_key = Vec::new();
    let mut base_transaction_id = envelope.base_transaction_id;
    for _ in 0..transaction_count {
        let entry_count = read_varint(&mut input)?;
        let mutation_count = usize::try_from(read_varint(&mut input)?).map_err(|_| {
            Error::Corrupt("write-ahead mutation count does not fit in memory".into())
        })?;
        if mutation_count > input.remaining() / 3 {
            return Err(Error::Corrupt(
                "write-ahead mutation count exceeds its payload".into(),
            ));
        }
        let changes = decode_changes(&mut input, mutation_count, &mut previous_key)?;
        let transaction_id = base_transaction_id
            .next()
            .map_err(|_| Error::Corrupt("write-ahead group transaction ID overflow".into()))?;
        records.push(WalRecord {
            kind: WalRecordKind::Transaction,
            base_transaction_id,
            transaction_id,
            entry_count,
            changes,
        });
        base_transaction_id = transaction_id;
    }
    if !input.is_empty() {
        return Err(Error::Corrupt(
            "write-ahead group payload has trailing bytes".into(),
        ));
    }
    if records.last().map(|record| record.entry_count) != Some(envelope.entry_count) {
        return Err(Error::Corrupt(
            "write-ahead group final entry count is invalid".into(),
        ));
    }
    Ok(records)
}

fn encoded_payload_len(records: &[WalRecord]) -> Result<usize> {
    let mut total = 0_usize;
    let mut previous_key: &[u8] = &[];
    for record in records {
        total = checked_add(total, varint_len(record.entry_count))?;
        total = checked_add(
            total,
            varint_len(u64::try_from(record.changes.len()).map_err(|_| Error::FileTooLarge)?),
        )?;
        for (key, value) in record.changes.sorted_iter() {
            let prefix_len = common_prefix_len(previous_key, key);
            let suffix_len = key.len().saturating_sub(prefix_len);
            total = checked_add(
                total,
                varint_len(u64::try_from(prefix_len).map_err(|_| Error::FileTooLarge)?),
            )?;
            total = checked_add(
                total,
                varint_len(u64::try_from(suffix_len).map_err(|_| Error::FileTooLarge)?),
            )?;
            total = checked_add(total, suffix_len)?;
            let value_tag = value.map_or(Ok(0), |value| {
                u64::try_from(value.len())
                    .map_err(|_| Error::FileTooLarge)?
                    .checked_add(1)
                    .ok_or(Error::FileTooLarge)
            })?;
            total = checked_add(total, varint_len(value_tag))?;
            total = checked_add(total, value.map_or(0, <[u8]>::len))?;
            previous_key = key;
        }
    }
    let total_u64 = u64::try_from(total).map_err(|_| Error::FileTooLarge)?;
    if total_u64 > MAX_RECORD_BYTES {
        return Err(Error::FileTooLarge);
    }
    Ok(total)
}

fn encode_payload(bytes: &mut Vec<u8>, records: &[WalRecord]) -> Result<()> {
    let mut previous_key: &[u8] = &[];
    for record in records {
        push_varint(bytes, record.entry_count);
        push_varint(
            bytes,
            u64::try_from(record.changes.len()).map_err(|_| Error::FileTooLarge)?,
        );
        for (key, value) in record.changes.sorted_iter() {
            let prefix_len = common_prefix_len(previous_key, key);
            let suffix = &key[prefix_len..];
            push_varint(
                bytes,
                u64::try_from(prefix_len).map_err(|_| Error::FileTooLarge)?,
            );
            push_varint(
                bytes,
                u64::try_from(suffix.len()).map_err(|_| Error::FileTooLarge)?,
            );
            bytes.extend_from_slice(suffix);
            let value_tag = value.map_or(Ok(0), |value| {
                u64::try_from(value.len())
                    .map_err(|_| Error::FileTooLarge)?
                    .checked_add(1)
                    .ok_or(Error::FileTooLarge)
            })?;
            push_varint(bytes, value_tag);
            if let Some(value) = value {
                bytes.extend_from_slice(value);
            }
            previous_key = key;
        }
    }
    Ok(())
}

fn decode_changes(
    input: &mut Input<'_>,
    mutation_count: usize,
    previous_key: &mut Vec<u8>,
) -> Result<ChangeSet> {
    let mut changes = ChangeSet::new();
    let mut previous_in_transaction: Option<Vec<u8>> = None;
    for _ in 0..mutation_count {
        let prefix_len = usize::try_from(read_varint(input)?).map_err(|_| {
            Error::Corrupt("write-ahead key prefix length does not fit in memory".into())
        })?;
        let suffix_len = usize::try_from(read_varint(input)?).map_err(|_| {
            Error::Corrupt("write-ahead key suffix length does not fit in memory".into())
        })?;
        if prefix_len > previous_key.len() {
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
        key.extend_from_slice(&previous_key[..prefix_len]);
        key.extend_from_slice(input.take(suffix_len)?);
        if previous_in_transaction
            .as_ref()
            .is_some_and(|previous| previous >= &key)
        {
            return Err(Error::Corrupt(
                "write-ahead keys are not strictly ordered".into(),
            ));
        }
        let value_tag = read_varint(input)?;
        let value = if value_tag == 0 {
            None
        } else {
            let value_len = usize::try_from(value_tag - 1).map_err(|_| {
                Error::Corrupt("write-ahead value length does not fit in memory".into())
            })?;
            if value_len > MAX_VALUE_BYTES {
                return Err(Error::Corrupt("write-ahead value is too large".into()));
            }
            Some(clone_bytes(input.take(value_len)?)?)
        };
        changes.insert(&key, value);
        previous_key.clear();
        previous_key.extend_from_slice(&key);
        previous_in_transaction = Some(key);
    }
    Ok(changes)
}

fn checked_add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right).ok_or(Error::FileTooLarge)
}

const fn varint_len(mut value: u64) -> usize {
    let mut bytes = 1;
    while value >= 0x80 {
        value >>= 7;
        bytes += 1;
    }
    bytes
}

fn push_varint(bytes: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        bytes.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn read_varint(input: &mut Input<'_>) -> Result<u64> {
    let mut value = 0_u64;
    for index in 0..10_u32 {
        let byte = input.take(1)?[0];
        if index == 9 && byte > 1 {
            return Err(Error::Corrupt("write-ahead varint overflows u64".into()));
        }
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            if index > 0 && byte == 0 {
                return Err(Error::Corrupt("write-ahead varint is not canonical".into()));
            }
            return Ok(value);
        }
    }
    Err(Error::Corrupt("write-ahead varint overflows u64".into()))
}
