use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use super::fault_injection;
use crate::{Error, Result};

pub(crate) struct LockFile {
    file: File,
    path: PathBuf,
}

impl LockFile {
    pub(crate) fn try_clone(&self) -> Result<Self> {
        Ok(Self {
            file: self.file.try_clone()?,
            path: self.path.clone(),
        })
    }
}

pub(crate) struct AdvisoryFileLock<'a> {
    file: &'a File,
}

impl<'a> AdvisoryFileLock<'a> {
    pub(crate) fn acquire(lock_file: &'a LockFile) -> Result<Self> {
        lock_file.file.lock()?;
        let lock = Self {
            file: &lock_file.file,
        };
        validate_lock_identity(&lock_file.path, &lock_file.file)?;
        Ok(lock)
    }
}

impl Drop for AdvisoryFileLock<'_> {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Long-lived exclusion for operations that write or replace the main file.
///
/// WAL commits intentionally do not take this lease. A background checkpoint
/// can therefore hold it while foreground commits continue to append to the
/// WAL. Callers that also need the short commit lock must acquire this lease
/// first, or use [`Self::try_acquire`] while already holding the commit lock.
pub(crate) struct MainFileLease {
    file: File,
}

impl MainFileLease {
    pub(crate) fn acquire(database: &Path) -> Result<Self> {
        let file = open_database(database)?;
        Self::acquire_opened(database, file)
    }

    fn acquire_opened(database: &Path, file: File) -> Result<Self> {
        file.lock()?;
        let lease = Self { file };
        lease.validate_current(database)?;
        Ok(lease)
    }

    pub(crate) fn try_acquire(database: &Path) -> Result<Option<Self>> {
        let file = open_database(database)?;
        match file.try_lock() {
            Ok(()) => {
                let lease = Self { file };
                lease.validate_current(database)?;
                Ok(Some(lease))
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    fn validate_current(&self, database: &Path) -> Result<()> {
        let current = open_database(database)?;
        if !same_file_identity(&self.file, &current)? {
            return Err(Error::WriteConflict);
        }
        Ok(())
    }
}

impl Drop for MainFileLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) fn open_lock_file(database: &Path) -> Result<LockFile> {
    let path = lock_path(database);
    validate_lock_path(&path, true)?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    validate_lock_identity(&path, &file)?;
    Ok(LockFile { file, path })
}

pub(crate) fn open_database(database: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .truncate(false)
        .open(database)
        .map_err(Into::into)
}

#[cfg(unix)]
pub(crate) fn same_file_identity(left: &File, right: &File) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let left = left.metadata()?;
    let right = right.metadata()?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(not(unix))]
pub(crate) fn same_file_identity(_left: &File, _right: &File) -> Result<bool> {
    // Active generation replacement is unsupported outside Unix. Keep the
    // portable core functional until an audited stable file-ID adapter exists.
    Ok(true)
}

#[cfg(unix)]
pub(crate) fn current_file_identity_matches(database: &Path, pinned: &File) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let pinned = pinned.metadata()?;
    let current = match std::fs::symlink_metadata(database) {
        Ok(current) => current,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !current.file_type().is_file() || pinned.nlink() != 1 || current.nlink() != 1 {
        return Ok(false);
    }
    Ok(pinned.dev() == current.dev() && pinned.ino() == current.ino())
}

#[cfg(not(unix))]
pub(crate) fn current_file_identity_matches(_database: &Path, _pinned: &File) -> Result<bool> {
    // Stable file IDs are platform-specific. Windows remains best effort until
    // its adapter can provide an audited equivalent of the Unix check.
    Ok(true)
}

pub(crate) fn normalize_database_path(database: &Path) -> Result<PathBuf> {
    match std::fs::canonicalize(database) {
        Ok(path) => {
            if !std::fs::symlink_metadata(&path)?.file_type().is_file() {
                return Err(crate::Error::InvalidInput(
                    "database path must refer to a regular file",
                ));
            }
            Ok(path)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(database).is_ok() {
                return Err(error.into());
            }
            let file_name = database.file_name().ok_or(crate::Error::InvalidInput(
                "database path must have a file name",
            ))?;
            let parent = database
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            Ok(std::fs::canonicalize(parent)?.join(file_name))
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
pub(crate) fn validate_database_identity(database: &Path, file: &File) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;

    let identity = file.metadata()?;
    if identity.nlink() == 1 {
        return Ok(());
    }
    let file_name = database.file_name().ok_or(crate::Error::InvalidInput(
        "database path must have a file name",
    ))?;
    let mut temporary_prefix = file_name.as_bytes().to_vec();
    temporary_prefix.extend_from_slice(b".wsldb-tmp-");
    let parent = database.parent().unwrap_or_else(|| Path::new("."));
    let mut removed = false;
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        if !entry.file_name().as_bytes().starts_with(&temporary_prefix)
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        let metadata = entry.metadata()?;
        if metadata.dev() == identity.dev() && metadata.ino() == identity.ino() {
            std::fs::remove_file(entry.path())?;
            removed = true;
        }
    }
    if removed {
        sync_parent_directory(database)?;
    }
    if file.metadata()?.nlink() != 1 {
        return Err(crate::Error::InvalidInput(
            "database files with hard links are not supported",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn validate_database_identity(_database: &Path, _file: &File) -> Result<()> {
    Ok(())
}

pub(crate) fn lock_path(database: &Path) -> PathBuf {
    let mut name = OsString::from(database.as_os_str());
    name.push(".lock");
    PathBuf::from(name)
}

fn validate_lock_path(path: &Path, missing_is_valid: bool) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => validate_lock_metadata(&metadata),
        Err(error) if missing_is_valid && error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn validate_lock_metadata(metadata: &std::fs::Metadata) -> Result<()> {
    if !metadata.file_type().is_file() {
        return Err(crate::Error::InvalidInput(
            "database lock sidecar must be a regular file, not a symbolic link",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.nlink() != 1 {
            return Err(crate::Error::InvalidInput(
                "database lock sidecar must not have hard links",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn validate_lock_identity(path: &Path, file: &File) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let open = file.metadata()?;
    if open.nlink() == 0 {
        return Err(crate::Error::InvalidInput(
            "database lock file was removed; reopen the database",
        ));
    }
    validate_lock_metadata(&open)?;
    let current = match std::fs::symlink_metadata(path) {
        Ok(current) => current,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(crate::Error::InvalidInput(
                "database lock sidecar identity changed; reopen the database",
            ));
        }
        Err(error) => return Err(error.into()),
    };
    validate_lock_metadata(&current)?;
    if open.dev() != current.dev() || open.ino() != current.ino() {
        return Err(crate::Error::InvalidInput(
            "database lock sidecar identity changed; reopen the database",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_lock_identity(path: &Path, file: &File) -> Result<()> {
    validate_lock_metadata(&file.metadata()?)?;
    validate_lock_path(path, false)
}

#[cfg(unix)]
pub(crate) fn replace_file(replacement: &Path, database: &Path) -> Result<()> {
    fault_injection::before("active_replace")?;
    std::fs::rename(replacement, database)?;
    super::crash_injection::hit("compaction_rename");
    fault_injection::before("active_replace_directory_sync")
        .map_err(Error::PublicationOutcomeUnknown)?;
    sync_parent_directory(database).map_err(|error| match error {
        Error::Io(error) => Error::PublicationOutcomeUnknown(error),
        other => other,
    })
}

#[cfg(not(unix))]
pub(crate) fn replace_file(_replacement: &Path, _database: &Path) -> Result<()> {
    Err(crate::Error::InvalidInput(
        "atomic active-file compaction is not supported on this platform",
    ))
}

pub(crate) fn sync_parent_directory(path: &Path) -> Result<()> {
    sync_directory(path.parent().unwrap_or_else(|| Path::new(".")))
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<()> {
    File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{AdvisoryFileLock, LockFile, MainFileLease};
    use crate::Error;

    static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn main_file_lease_excludes_independent_open_files() {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "wsldb-{}-{sequence}.main-file-lease",
            std::process::id()
        ));
        std::fs::write(&path, b"lease").unwrap();

        let first = MainFileLease::try_acquire(&path).unwrap().unwrap();
        assert!(MainFileLease::try_acquire(&path).unwrap().is_none());
        drop(first);
        assert!(MainFileLease::try_acquire(&path).unwrap().is_some());

        std::fs::remove_file(&path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn main_file_lease_rejects_a_descriptor_opened_before_replacement() {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "wsldb-{}-{sequence}.stale-main-file-lease",
            std::process::id()
        ));
        let replacement = path.with_extension("main-file-lease-replacement");
        std::fs::write(&path, b"old generation").unwrap();
        let stale = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        std::fs::write(&replacement, b"new generation").unwrap();
        std::fs::rename(&replacement, &path).unwrap();

        assert!(matches!(
            MainFileLease::acquire_opened(&path, stale),
            Err(Error::WriteConflict)
        ));

        std::fs::remove_file(&path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unlinked_sidecar_fails_before_using_a_stale_lock_inode() {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "wsldb-{}-{sequence}.unlinked-sidecar",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();

        assert!(matches!(
            AdvisoryFileLock::acquire(&LockFile { file, path }),
            Err(Error::InvalidInput(
                "database lock file was removed; reopen the database"
            ))
        ));
    }
}
