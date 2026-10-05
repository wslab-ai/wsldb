use super::{PageId, TransactionId};

/// Durable root of one committed tree version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Head {
    transaction_id: TransactionId,
    root: Option<PageId>,
    last_page: u64,
    entry_count: u64,
}

impl Head {
    pub(crate) const fn empty() -> Self {
        Self {
            transaction_id: TransactionId::INITIAL,
            root: None,
            last_page: 1,
            entry_count: 0,
        }
    }

    pub(crate) const fn new(
        transaction_id: TransactionId,
        root: Option<PageId>,
        last_page: u64,
        entry_count: u64,
    ) -> Self {
        Self {
            transaction_id,
            root,
            last_page,
            entry_count,
        }
    }

    pub(crate) const fn transaction_id(self) -> TransactionId {
        self.transaction_id
    }

    pub(crate) const fn root(self) -> Option<PageId> {
        self.root
    }

    pub(crate) const fn last_page(self) -> u64 {
        self.last_page
    }

    pub(crate) const fn entry_count(self) -> u64 {
        self.entry_count
    }
}
