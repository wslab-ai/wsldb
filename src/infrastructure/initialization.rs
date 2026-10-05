use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

use super::crash_injection;
use super::fault_injection;
use super::file_context::load_head;
use super::metadata::{self, METADATA_REGION_SIZE};
use super::path_lock::open_database;
use super::positioned_io::write_all_at;
use super::snapshot_file;
use crate::domain::Head;
use crate::{Error, Result};

pub(crate) struct InitializedDatabase {
    pub(crate) file: File,
    pub(crate) head: Head,
}

pub(crate) fn open_or_create(path: &Path) -> Result<InitializedDatabase> {
    match open_database(path) {
        Ok(file) => open_existing(file),
        Err(Error::Io(error)) if error.kind() == ErrorKind::NotFound => create(path),
        Err(error) => Err(error),
    }
}

fn open_existing(file: File) -> Result<InitializedDatabase> {
    if file.metadata()?.len() == 0 {
        return Err(Error::Corrupt("database file is empty".into()));
    }
    let head = load_head(&file)?;
    Ok(InitializedDatabase { file, head })
}

fn create(path: &Path) -> Result<InitializedDatabase> {
    let (file, temporary) = snapshot_file::create_temporary(path)?;
    file.set_len(METADATA_REGION_SIZE)?;
    let head = Head::empty();
    write_all_at(
        &file,
        &metadata::encode(head),
        metadata::slot_offset(head.transaction_id()),
    )?;
    crash_injection::hit("initial_metadata_write");
    fault_injection::before("initial_metadata_sync")?;
    file.sync_all()?;
    crash_injection::hit("initial_metadata_sync");

    snapshot_file::publish(file.try_clone()?, temporary, path)?;
    Ok(InitializedDatabase { file, head })
}
