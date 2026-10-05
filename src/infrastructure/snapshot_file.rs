use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::fault_injection;
use super::metadata::{self, METADATA_REGION_SIZE};
use super::path_lock::sync_parent_directory;
use super::positioned_io::{read_exact_at, write_all_at};
use crate::domain::Head;
use crate::{Error, Result};

static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);
const COPY_BUFFER_BYTES: usize = 64 * 1024;

pub(crate) struct TemporarySnapshot {
    path: Option<std::path::PathBuf>,
}

impl Drop for TemporarySnapshot {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub(crate) fn create_temporary(target: &Path) -> Result<(File, TemporarySnapshot)> {
    let file_name = target
        .file_name()
        .ok_or(Error::InvalidInput("snapshot target must have a file name"))?;
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    for _ in 0..100 {
        let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(file_name);
        temporary_name.push(format!(".wsldb-tmp-{}-{sequence}", std::process::id()));
        let path = parent.join(temporary_name);
        match OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => {
                return Ok((file, TemporarySnapshot { path: Some(path) }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a temporary snapshot path",
    )))
}

pub(crate) fn replacement_path(target: &Path) -> Result<PathBuf> {
    let file_name = target
        .file_name()
        .ok_or(Error::InvalidInput("database path must have a file name"))?;
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    for _ in 0..100 {
        let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
        let mut replacement_name = OsString::from(file_name);
        replacement_name.push(format!(
            ".wsldb-replacement-{}-{sequence}",
            std::process::id()
        ));
        let path = parent.join(replacement_name);
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
            Ok(_) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate an active-compaction replacement path",
    )))
}

pub(crate) fn copy(source: &File, destination: &File, head: Head, bytes: u64) -> Result<()> {
    write_all_at(
        destination,
        &metadata::encode(head),
        metadata::slot_offset(head.transaction_id()),
    )?;
    super::crash_injection::hit("snapshot_copy_write");
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    let mut offset = METADATA_REGION_SIZE;
    while offset < bytes {
        let remaining = bytes - offset;
        let chunk = usize::try_from(remaining.min(COPY_BUFFER_BYTES as u64))
            .map_err(|_| Error::FileTooLarge)?;
        read_exact_at(source, &mut buffer[..chunk], offset)?;
        write_all_at(destination, &buffer[..chunk], offset)?;
        super::crash_injection::hit("snapshot_copy_write");
        offset = offset
            .checked_add(u64::try_from(chunk).map_err(|_| Error::FileTooLarge)?)
            .ok_or(Error::FileTooLarge)?;
    }
    destination.set_len(bytes)?;
    Ok(())
}

pub(crate) fn publish(
    destination: File,
    mut temporary: TemporarySnapshot,
    target: &Path,
) -> Result<()> {
    drop(destination);
    let temporary_path = temporary.path.as_ref().ok_or(Error::InternalInvariant(
        "temporary snapshot path is missing",
    ))?;
    fault_injection::before("snapshot_publish_link")?;
    std::fs::hard_link(temporary_path, target)?;
    super::crash_injection::hit("snapshot_publish_link");
    fault_injection::before("snapshot_publish_directory_sync")
        .map_err(Error::PublicationOutcomeUnknown)?;
    sync_parent_directory(target).map_err(publication_error)?;
    super::crash_injection::hit("snapshot_publish_directory_sync");
    fault_injection::before("snapshot_temporary_remove")
        .map_err(Error::PublicationOutcomeUnknown)?;
    std::fs::remove_file(temporary_path).map_err(Error::PublicationOutcomeUnknown)?;
    temporary.path = None;
    super::crash_injection::hit("snapshot_temporary_remove");
    fault_injection::before("snapshot_cleanup_directory_sync")
        .map_err(Error::PublicationOutcomeUnknown)?;
    sync_parent_directory(target).map_err(publication_error)?;
    super::crash_injection::hit("snapshot_cleanup_directory_sync");
    Ok(())
}

fn publication_error(error: Error) -> Error {
    match error {
        Error::Io(error) => Error::PublicationOutcomeUnknown(error),
        other => other,
    }
}
