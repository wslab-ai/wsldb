use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::path;
use crate::allocation::zeroed_bytes;
use crate::infrastructure::fault_injection;
use crate::infrastructure::path_lock::sync_parent_directory;
use crate::infrastructure::positioned_io::{read_exact_at, write_all_at};
use crate::{Error, Result};

const COPY_BUFFER_BYTES: usize = 256 * 1024;
static NEXT_HANDOFF: AtomicU64 = AtomicU64::new(0);

pub(crate) struct WalHandoff {
    database: PathBuf,
    temporary: Option<PathBuf>,
    file: File,
    copied_bytes: u64,
}

impl WalHandoff {
    pub(crate) fn stage(database: &Path, source: &File, bytes: u64) -> Result<Self> {
        if source.metadata()?.len() < bytes {
            return Err(Error::WriteConflict);
        }
        let wal_path = path(database);
        let (temporary, file) = create_temporary(&wal_path)?;
        let mut handoff = Self {
            database: database.to_path_buf(),
            temporary: Some(temporary),
            file,
            copied_bytes: 0,
        };
        handoff.copy_tail(source, bytes)?;
        handoff.file.set_len(bytes)?;
        fault_injection::before("compaction_wal_stage_sync")?;
        handoff.file.sync_all()?;
        super::super::crash_injection::hit("compaction_wal_stage_sync");
        Ok(handoff)
    }

    pub(crate) fn publish(mut self, source: &File, bytes: u64) -> Result<()> {
        if bytes < self.copied_bytes || source.metadata()?.len() != bytes {
            return Err(Error::WriteConflict);
        }
        self.copy_tail(source, bytes)?;
        self.file.set_len(bytes)?;
        fault_injection::before("compaction_wal_handoff_sync")?;
        self.file.sync_all()?;
        super::super::crash_injection::hit("compaction_wal_handoff_sync");

        let temporary = self.temporary.take().ok_or(Error::InternalInvariant(
            "WAL handoff temporary path is missing",
        ))?;
        let wal_path = path(&self.database);
        fault_injection::before("compaction_wal_handoff_rename")?;
        std::fs::rename(&temporary, &wal_path)?;
        super::super::crash_injection::hit("compaction_wal_handoff_rename");
        fault_injection::before("compaction_wal_handoff_directory_sync")
            .map_err(Error::PublicationOutcomeUnknown)?;
        sync_parent_directory(&wal_path).map_err(publication_error)?;
        super::super::crash_injection::hit("compaction_wal_handoff_directory_sync");
        Ok(())
    }

    fn copy_tail(&mut self, source: &File, end: u64) -> Result<()> {
        let mut buffer = zeroed_bytes(COPY_BUFFER_BYTES)?;
        while self.copied_bytes < end {
            let remaining = end - self.copied_bytes;
            let chunk = usize::try_from(remaining.min(COPY_BUFFER_BYTES as u64))
                .map_err(|_| Error::FileTooLarge)?;
            read_exact_at(source, &mut buffer[..chunk], self.copied_bytes)?;
            fault_injection::before("compaction_wal_handoff_write")?;
            write_all_at(&self.file, &buffer[..chunk], self.copied_bytes)?;
            super::super::crash_injection::hit("compaction_wal_handoff_write");
            self.copied_bytes = self
                .copied_bytes
                .checked_add(u64::try_from(chunk).map_err(|_| Error::FileTooLarge)?)
                .ok_or(Error::FileTooLarge)?;
        }
        Ok(())
    }
}

impl Drop for WalHandoff {
    fn drop(&mut self) {
        if let Some(path) = self.temporary.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn create_temporary(wal_path: &Path) -> Result<(PathBuf, File)> {
    let file_name = wal_path.file_name().ok_or(Error::InvalidInput(
        "write-ahead path must have a file name",
    ))?;
    let parent = wal_path.parent().unwrap_or_else(|| Path::new("."));
    for _ in 0..100 {
        let sequence = NEXT_HANDOFF.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(file_name);
        name.push(format!(".wsldb-handoff-{}-{sequence}", std::process::id()));
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
        "could not allocate a WAL handoff file",
    )))
}

fn publication_error(error: Error) -> Error {
    match error {
        Error::Io(error) => Error::PublicationOutcomeUnknown(error),
        other => other,
    }
}
