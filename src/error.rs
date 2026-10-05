use std::fmt::{self, Display, Formatter};
use std::io;

/// Errors returned by WSLDB operations.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A filesystem operation failed.
    Io(io::Error),
    /// Commit publication began, but an I/O failure prevented confirmation.
    ///
    /// The commit may become visible after refresh or reopen. Callers must
    /// inspect durable state before deciding whether to retry the transaction.
    CommitOutcomeUnknown(io::Error),
    /// A verified file was linked or renamed, but publication was not fully
    /// confirmed.
    ///
    /// The target may already contain the new snapshot or generation. Inspect
    /// the target and refresh or reopen active database handles before retrying.
    PublicationOutcomeUnknown(io::Error),
    /// The durable state failed validation.
    Corrupt(String),
    /// An input violates a public database invariant.
    InvalidInput(&'static str),
    /// Another database handle committed after this writer began.
    WriteConflict,
    /// The transaction ID space is exhausted.
    TransactionIdExhausted,
    /// A corrupt tree exceeded the maximum supported traversal depth.
    TreeDepthExceeded,
    /// An internal tree invariant was violated.
    InternalInvariant(&'static str),
    /// Internal synchronization state was poisoned by a panic.
    LockPoisoned(&'static str),
    /// The database file cannot represent another page.
    FileTooLarge,
}

impl Error {
    pub(crate) fn duplicate(&self) -> Self {
        match self {
            Self::Io(error) => Self::Io(duplicate_io(error)),
            Self::CommitOutcomeUnknown(error) => Self::CommitOutcomeUnknown(duplicate_io(error)),
            Self::PublicationOutcomeUnknown(error) => {
                Self::PublicationOutcomeUnknown(duplicate_io(error))
            }
            Self::Corrupt(reason) => Self::Corrupt(reason.clone()),
            Self::InvalidInput(reason) => Self::InvalidInput(reason),
            Self::WriteConflict => Self::WriteConflict,
            Self::TransactionIdExhausted => Self::TransactionIdExhausted,
            Self::TreeDepthExceeded => Self::TreeDepthExceeded,
            Self::InternalInvariant(reason) => Self::InternalInvariant(reason),
            Self::LockPoisoned(lock) => Self::LockPoisoned(lock),
            Self::FileTooLarge => Self::FileTooLarge,
        }
    }
}

fn duplicate_io(error: &io::Error) -> io::Error {
    error.raw_os_error().map_or_else(
        || io::Error::new(error.kind(), error.to_string()),
        io::Error::from_raw_os_error,
    )
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::CommitOutcomeUnknown(error) => {
                write!(
                    formatter,
                    "commit outcome is unknown after publication I/O failure: {error}"
                )
            }
            Self::PublicationOutcomeUnknown(error) => {
                write!(
                    formatter,
                    "snapshot publication outcome is unknown after I/O failure: {error}"
                )
            }
            Self::Corrupt(reason) => write!(formatter, "database is corrupt: {reason}"),
            Self::InvalidInput(reason) => write!(formatter, "invalid input: {reason}"),
            Self::WriteConflict => {
                formatter.write_str("another database handle committed concurrently")
            }
            Self::TransactionIdExhausted => formatter.write_str("transaction ID space exhausted"),
            Self::TreeDepthExceeded => formatter.write_str("tree depth exceeds the safety limit"),
            Self::InternalInvariant(reason) => {
                write!(formatter, "internal invariant failed: {reason}")
            }
            Self::LockPoisoned(lock) => write!(formatter, "database {lock} lock was poisoned"),
            Self::FileTooLarge => formatter.write_str("database file is too large"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error)
            | Self::CommitOutcomeUnknown(error)
            | Self::PublicationOutcomeUnknown(error) => Some(error),
            Self::Corrupt(_)
            | Self::InvalidInput(_)
            | Self::WriteConflict
            | Self::TransactionIdExhausted
            | Self::TreeDepthExceeded
            | Self::InternalInvariant(_)
            | Self::LockPoisoned(_)
            | Self::FileTooLarge => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The result type used by WSLDB.
pub type Result<T> = std::result::Result<T, Error>;
