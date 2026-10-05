use crate::{Error, Result};

/// Monotonically increasing identifier of a committed database snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct TransactionId(u64);

impl TransactionId {
    pub(crate) const INITIAL: Self = Self(0);

    pub(crate) const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric transaction identifier.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    pub(crate) fn next(self) -> Result<Self> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(Error::TransactionIdExhausted)
    }
}
