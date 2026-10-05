use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::Instant;

use super::{
    FOOTER_BYTES, FOOTER_MAGIC, HEADER_BYTES, MAX_RECORD_BYTES, SNAPSHOT_MAGIC,
    SnapshotReplacement, TOMBSTONE_LENGTH, WalRecordKind, common_prefix_len, path,
    validate_sequence,
};
use crate::domain::{MAX_KEY_BYTES, MAX_VALUE_BYTES};
use crate::infrastructure::checksum::{Crc32c, crc32c, crc32c_combine};
use crate::infrastructure::path_lock::sync_parent_directory;
use crate::infrastructure::positioned_io::write_all_at;
use crate::infrastructure::write_buffer::WriteBuffer;
use crate::{Error, Result, TransactionId};

const STREAM_BUFFER_BYTES: usize = 256 * 1024;
#[cfg(unix)]
static NEXT_REPLACEMENT: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
pub(crate) fn replace_with_snapshot_streamed(
    database: &Path,
    base_transaction_id: TransactionId,
    transaction_id: TransactionId,
    entry_count: u64,
    buffer: &WriteBuffer,
) -> Result<Option<SnapshotReplacement>> {
    validate_sequence(WalRecordKind::Snapshot, base_transaction_id, transaction_id)
        .map_err(|_| Error::InternalInvariant("write-ahead transaction sequence"))?;
    let summary = visit_entries(buffer, |_| Ok(()))?;
    let total_len = HEADER_BYTES
        .checked_add(summary.payload_len)
        .and_then(|bytes| bytes.checked_add(FOOTER_BYTES))
        .ok_or(Error::FileTooLarge)?;
    let total_len_u64 = u64::try_from(total_len).map_err(|_| Error::FileTooLarge)?;
    if total_len_u64 > MAX_RECORD_BYTES {
        return Err(Error::FileTooLarge);
    }
    let header = snapshot_header(
        total_len_u64,
        base_transaction_id,
        transaction_id,
        entry_count,
        summary,
    );
    let footer = snapshot_footer(&header, summary, total_len_u64);

    publish(database, total_len_u64, |file| {
        let mut writer = StagedWriter::new(file)?;
        writer.write(&header)?;
        let written = visit_entries(buffer, |bytes| writer.write(bytes))?;
        if written != summary {
            return Err(Error::InternalInvariant(
                "write-ahead snapshot changed during streaming",
            ));
        }
        writer.write(&footer)?;
        if writer.finish()? != total_len_u64 {
            return Err(Error::InternalInvariant(
                "write-ahead snapshot length changed during streaming",
            ));
        }
        Ok(())
    })
    .map(Some)
}

#[cfg(not(unix))]
pub(crate) fn replace_with_snapshot_streamed(
    _database: &Path,
    _base_transaction_id: TransactionId,
    _transaction_id: TransactionId,
    _entry_count: u64,
    _buffer: &WriteBuffer,
) -> Result<Option<SnapshotReplacement>> {
    Ok(None)
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct EntrySummary {
    mutations: u32,
    payload_len: usize,
    payload_crc: u32,
}

fn visit_entries(
    buffer: &WriteBuffer,
    mut visit: impl FnMut(&[u8]) -> Result<()>,
) -> Result<EntrySummary> {
    let mut mutations = 0_u32;
    let mut payload_len = 0_usize;
    let mut payload_crc = Crc32c::new();
    let mut previous_key = Vec::new();
    buffer.visit_latest(|key, value| {
        if key.is_empty() || key.len() > MAX_KEY_BYTES {
            return Err(Error::InternalInvariant(
                "write-ahead snapshot key length is invalid",
            ));
        }
        let prefix_len = common_prefix_len(&previous_key, key);
        let suffix = &key[prefix_len..];
        let prefix_len = u16::try_from(prefix_len).map_err(|_| Error::FileTooLarge)?;
        let suffix_len = u16::try_from(suffix.len()).map_err(|_| Error::FileTooLarge)?;
        let value_len = match value {
            Some(value) if value.len() <= MAX_VALUE_BYTES => {
                u64::try_from(value.len()).map_err(|_| Error::FileTooLarge)?
            }
            Some(_) => return Err(Error::FileTooLarge),
            None => TOMBSTONE_LENGTH,
        };
        let prefix_len = prefix_len.to_le_bytes();
        let suffix_len = suffix_len.to_le_bytes();
        let value_len = value_len.to_le_bytes();
        for bytes in [&prefix_len[..], &suffix_len[..], suffix, &value_len[..]] {
            payload_len = payload_len
                .checked_add(bytes.len())
                .ok_or(Error::FileTooLarge)?;
            payload_crc.update(bytes);
            visit(bytes)?;
        }
        if let Some(value) = value {
            payload_len = payload_len
                .checked_add(value.len())
                .ok_or(Error::FileTooLarge)?;
            payload_crc.update(value);
            visit(value)?;
        }
        previous_key.clear();
        previous_key.extend_from_slice(key);
        mutations = mutations.checked_add(1).ok_or(Error::FileTooLarge)?;
        Ok(())
    })?;
    Ok(EntrySummary {
        mutations,
        payload_len,
        payload_crc: payload_crc.finish(),
    })
}

fn snapshot_header(
    total_len: u64,
    base_transaction_id: TransactionId,
    transaction_id: TransactionId,
    entry_count: u64,
    summary: EntrySummary,
) -> [u8; HEADER_BYTES] {
    let mut header = [0_u8; HEADER_BYTES];
    header[..8].copy_from_slice(&SNAPSHOT_MAGIC);
    header[8..16].copy_from_slice(&total_len.to_le_bytes());
    header[16..24].copy_from_slice(&base_transaction_id.value().to_le_bytes());
    header[24..32].copy_from_slice(&transaction_id.value().to_le_bytes());
    header[32..40].copy_from_slice(&entry_count.to_le_bytes());
    header[40..44].copy_from_slice(&summary.mutations.to_le_bytes());
    header[44..48].copy_from_slice(&summary.payload_crc.to_le_bytes());
    let header_crc = crc32c(&header[..48]);
    header[48..52].copy_from_slice(&header_crc.to_le_bytes());
    header
}

fn snapshot_footer(
    header: &[u8; HEADER_BYTES],
    summary: EntrySummary,
    total_len: u64,
) -> [u8; FOOTER_BYTES] {
    let mut footer = [0_u8; FOOTER_BYTES];
    let record_crc = crc32c_combine(crc32c(header), summary.payload_crc, summary.payload_len);
    footer[..4].copy_from_slice(&record_crc.to_le_bytes());
    footer[4..8].copy_from_slice(&FOOTER_MAGIC);
    footer[8..16].copy_from_slice(&total_len.to_le_bytes());
    footer
}

#[cfg(unix)]
fn publish(
    database: &Path,
    bytes: u64,
    write: impl FnOnce(&File) -> Result<()>,
) -> Result<SnapshotReplacement> {
    let wal_path = path(database);
    let (replacement_path, replacement) = create_replacement(&wal_path)?;
    let publication = (|| {
        write(&replacement)?;
        replacement.set_len(bytes)?;
        let file_sync_started = Instant::now();
        super::fault_injection::before("write_buffer_snapshot_sync")?;
        replacement.sync_all()?;
        let mut sync = file_sync_started.elapsed();
        super::super::crash_injection::hit("write_buffer_snapshot_sync");
        super::fault_injection::before("write_buffer_snapshot_rename")?;
        std::fs::rename(&replacement_path, &wal_path)?;
        super::super::crash_injection::hit("write_buffer_snapshot_rename");
        let directory_sync_started = Instant::now();
        super::fault_injection::before("write_buffer_snapshot_directory_sync")
            .map_err(Error::PublicationOutcomeUnknown)?;
        sync_parent_directory(&wal_path).map_err(|error| match error {
            Error::Io(error) => Error::PublicationOutcomeUnknown(error),
            other => other,
        })?;
        sync = sync.saturating_add(directory_sync_started.elapsed());
        super::super::crash_injection::hit("write_buffer_snapshot_directory_sync");
        Ok(SnapshotReplacement {
            file: replacement,
            bytes,
            sync,
        })
    })();
    if publication.is_err() {
        let _ = std::fs::remove_file(replacement_path);
    }
    publication
}

#[cfg(unix)]
fn create_replacement(wal_path: &Path) -> Result<(PathBuf, File)> {
    let file_name = wal_path.file_name().ok_or(Error::InvalidInput(
        "write-ahead path must have a file name",
    ))?;
    let parent = wal_path.parent().unwrap_or_else(|| Path::new("."));
    for _ in 0..100 {
        let sequence = NEXT_REPLACEMENT.fetch_add(1, Ordering::Relaxed);
        let mut name = file_name.to_os_string();
        name.push(format!(
            ".wsldb-replacement-{}-{sequence}",
            std::process::id()
        ));
        let path = parent.join(name);
        match OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a write-ahead replacement path",
    )))
}

struct StagedWriter<'a> {
    file: &'a File,
    bytes: Vec<u8>,
    offset: u64,
}

impl<'a> StagedWriter<'a> {
    fn new(file: &'a File) -> Result<Self> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(STREAM_BUFFER_BYTES)
            .map_err(|_| Error::FileTooLarge)?;
        Ok(Self {
            file,
            bytes,
            offset: 0,
        })
    }

    fn write(&mut self, mut input: &[u8]) -> Result<()> {
        while !input.is_empty() {
            let available = STREAM_BUFFER_BYTES.saturating_sub(self.bytes.len());
            if available == 0 {
                self.flush()?;
                continue;
            }
            let copied = available.min(input.len());
            self.bytes.extend_from_slice(&input[..copied]);
            input = &input[copied..];
        }
        Ok(())
    }

    fn finish(mut self) -> Result<u64> {
        self.flush()?;
        Ok(self.offset)
    }

    fn flush(&mut self) -> Result<()> {
        if self.bytes.is_empty() {
            return Ok(());
        }
        write_all_at(self.file, &self.bytes, self.offset)?;
        super::super::crash_injection::hit("write_buffer_snapshot_write");
        self.offset = self
            .offset
            .checked_add(u64::try_from(self.bytes.len()).map_err(|_| Error::FileTooLarge)?)
            .ok_or(Error::FileTooLarge)?;
        self.bytes.clear();
        Ok(())
    }
}
