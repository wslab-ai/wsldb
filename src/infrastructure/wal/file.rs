use std::fs::{File, Metadata, OpenOptions};
use std::path::{Path, PathBuf};

use super::super::fault_injection;
use super::super::path_lock::sync_parent_directory;
use crate::{Error, Result};

pub(crate) fn path(database: &Path) -> PathBuf {
    let mut path = database.as_os_str().to_owned();
    path.push(".wal");
    PathBuf::from(path)
}

pub(crate) fn open(database: &Path) -> Result<File> {
    let wal_path = path(database);
    let existing = validated_metadata(&wal_path)?;
    let needs_directory_sync = existing.as_ref().is_none_or(|metadata| metadata.len() == 0);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&wal_path)?;
    if !is_current_path(&file, &wal_path)? {
        return Err(Error::InvalidInput(
            "write-ahead sidecar identity changed during open",
        ));
    }
    if needs_directory_sync {
        fault_injection::before("wal_sidecar_directory_sync")?;
        sync_parent_directory(&wal_path)?;
    }
    Ok(file)
}

pub(crate) fn file_len(database: &Path) -> Result<u64> {
    Ok(validated_metadata(&path(database))?.map_or(0, |metadata| metadata.len()))
}

pub(crate) fn is_current(file: &File, database: &Path) -> Result<bool> {
    is_current_path(file, &path(database))
}

#[cfg(unix)]
fn is_current_path(file: &File, wal_path: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let open = file.metadata()?;
    validate_file_type(&open)?;
    if open.nlink() == 0 {
        return Ok(false);
    }
    if open.nlink() != 1 {
        return Err(Error::InvalidInput(
            "write-ahead sidecar must not have hard links",
        ));
    }
    let Some(current) = validated_metadata(wal_path)? else {
        return Ok(false);
    };
    Ok(open.dev() == current.dev() && open.ino() == current.ino())
}

#[cfg(not(unix))]
fn is_current_path(file: &File, wal_path: &Path) -> Result<bool> {
    validate_metadata(&file.metadata()?)?;
    Ok(validated_metadata(wal_path)?.is_some())
}

fn validated_metadata(wal_path: &Path) -> Result<Option<Metadata>> {
    match std::fs::symlink_metadata(wal_path) {
        Ok(metadata) => {
            validate_metadata(&metadata)?;
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn validate_metadata(metadata: &Metadata) -> Result<()> {
    validate_file_type(metadata)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.nlink() != 1 {
            return Err(Error::InvalidInput(
                "write-ahead sidecar must not have hard links",
            ));
        }
    }
    Ok(())
}

fn validate_file_type(metadata: &Metadata) -> Result<()> {
    if !metadata.file_type().is_file() {
        return Err(Error::InvalidInput(
            "write-ahead sidecar must be a regular file, not a symbolic link",
        ));
    }
    Ok(())
}
