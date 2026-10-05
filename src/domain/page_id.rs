/// Physical page identifier within a database file.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct PageId(u64);

impl PageId {
    pub(crate) const FIRST_DATA_PAGE: u64 = 2;

    pub(crate) const fn from_raw(value: u64) -> Option<Self> {
        if value >= Self::FIRST_DATA_PAGE {
            Some(Self(value))
        } else {
            None
        }
    }

    pub(crate) const fn value(self) -> u64 {
        self.0
    }
}
