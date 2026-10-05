use std::cmp::Ordering;

use super::page::{LeafEntry, LeafValue};
use crate::domain::{MAX_INLINE_VALUE_BYTES, MAX_KEY_BYTES, MAX_VALUE_BYTES, PageId};
use crate::{Error, Result};

const OVERFLOW_FLAG: u16 = 1 << 15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LeafSlot {
    suffix_start: u16,
    suffix_len: u16,
    value_start: u16,
    value_len: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OverflowValue {
    head: PageId,
    len: u64,
    checksum: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PackedLeaf {
    bytes: Box<[u8]>,
    prefix_len: u16,
    encoded_len: u16,
    slots: Box<[LeafSlot]>,
    overflows: Box<[OverflowValue]>,
}

pub(crate) enum PackedValue<'a> {
    Inline(&'a [u8]),
    Overflow {
        head: PageId,
        len: u64,
        checksum: u32,
    },
}

pub(crate) struct PackedLeafBuilder {
    bytes: Vec<u8>,
    prefix_len: u16,
    encoded_len: usize,
    slots: Vec<LeafSlot>,
    overflows: Vec<OverflowValue>,
}

impl PackedLeafBuilder {
    pub(crate) fn new(prefix: &[u8], capacity: usize) -> Result<Self> {
        let prefix_len = u16::try_from(prefix.len())
            .map_err(|_| Error::Corrupt("leaf key prefix is too long".into()))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(prefix.len())
            .map_err(|_| Error::FileTooLarge)?;
        bytes.extend_from_slice(prefix);
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(capacity)
            .map_err(|_| Error::FileTooLarge)?;
        Ok(Self {
            bytes,
            prefix_len,
            encoded_len: 4 + prefix.len(),
            slots,
            overflows: Vec::new(),
        })
    }

    pub(crate) fn push_inline(&mut self, suffix: &[u8], value: &[u8]) -> Result<()> {
        let suffix_start = compact_offset(self.bytes.len())?;
        let suffix_len = compact_length(suffix.len(), "leaf key suffix")?;
        self.bytes.extend_from_slice(suffix);
        let value_start = compact_offset(self.bytes.len())?;
        let value_len = compact_length(value.len(), "inline value")?;
        if value_len & OVERFLOW_FLAG != 0 {
            return Err(Error::Corrupt("inline value is too long".into()));
        }
        self.bytes.extend_from_slice(value);
        self.encoded_len = self
            .encoded_len
            .checked_add(2 + suffix.len() + 1 + 4 + value.len())
            .ok_or(Error::FileTooLarge)?;
        self.slots.push(LeafSlot {
            suffix_start,
            suffix_len,
            value_start,
            value_len,
        });
        Ok(())
    }

    pub(crate) fn push_overflow(
        &mut self,
        suffix: &[u8],
        head: PageId,
        len: u64,
        checksum: u32,
    ) -> Result<()> {
        let suffix_start = compact_offset(self.bytes.len())?;
        let suffix_len = compact_length(suffix.len(), "leaf key suffix")?;
        self.bytes.extend_from_slice(suffix);
        let overflow_index = compact_offset(self.overflows.len())?;
        self.overflows.push(OverflowValue {
            head,
            len,
            checksum,
        });
        self.encoded_len = self
            .encoded_len
            .checked_add(2 + suffix.len() + 1 + 8 + 8 + 4)
            .ok_or(Error::FileTooLarge)?;
        self.slots.push(LeafSlot {
            suffix_start,
            suffix_len,
            value_start: overflow_index,
            value_len: OVERFLOW_FLAG,
        });
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<PackedLeaf> {
        Ok(PackedLeaf {
            bytes: self.bytes.into_boxed_slice(),
            prefix_len: self.prefix_len,
            encoded_len: u16::try_from(self.encoded_len)
                .map_err(|_| Error::Corrupt("leaf payload is too large".into()))?,
            slots: self.slots.into_boxed_slice(),
            overflows: self.overflows.into_boxed_slice(),
        })
    }
}

impl PackedLeaf {
    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }

    pub(crate) const fn encoded_len(&self) -> usize {
        self.encoded_len as usize
    }

    pub(crate) fn prefix(&self) -> Result<&[u8]> {
        self.bytes
            .get(..usize::from(self.prefix_len))
            .ok_or(Error::InternalInvariant("packed leaf prefix"))
    }

    pub(crate) fn suffix(&self, index: usize) -> Result<&[u8]> {
        let slot = self.slot(index)?;
        packed_range(&self.bytes, slot.suffix_start, slot.suffix_len)
    }

    pub(crate) fn compare_key(&self, index: usize, key: &[u8]) -> Result<Ordering> {
        let prefix = self.prefix()?;
        let compared = prefix.len().min(key.len());
        match prefix[..compared].cmp(&key[..compared]) {
            Ordering::Equal if key.len() < prefix.len() => Ok(Ordering::Greater),
            Ordering::Equal => Ok(self.suffix(index)?.cmp(&key[prefix.len()..])),
            ordering => Ok(ordering),
        }
    }

    pub(crate) fn key(&self, index: usize) -> Result<Vec<u8>> {
        let prefix = self.prefix()?;
        let suffix = self.suffix(index)?;
        let mut key = Vec::with_capacity(prefix.len() + suffix.len());
        key.extend_from_slice(prefix);
        key.extend_from_slice(suffix);
        Ok(key)
    }

    pub(crate) fn value(&self, index: usize) -> Result<PackedValue<'_>> {
        let slot = self.slot(index)?;
        if slot.value_len & OVERFLOW_FLAG == 0 {
            return packed_range(&self.bytes, slot.value_start, slot.value_len)
                .map(PackedValue::Inline);
        }
        let overflow = self
            .overflows
            .get(usize::from(slot.value_start))
            .ok_or(Error::InternalInvariant("packed overflow index"))?;
        Ok(PackedValue::Overflow {
            head: overflow.head,
            len: overflow.len,
            checksum: overflow.checksum,
        })
    }

    pub(crate) fn find(&self, key: &[u8]) -> Result<std::result::Result<usize, usize>> {
        let mut left = 0;
        let mut right = self.len();
        while left < right {
            let middle = left + (right - left) / 2;
            match self.compare_key(middle, key)? {
                Ordering::Less => left = middle + 1,
                Ordering::Greater => right = middle,
                Ordering::Equal => return Ok(Ok(middle)),
            }
        }
        Ok(Err(left))
    }

    pub(crate) fn to_entries(&self) -> Result<Vec<LeafEntry>> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(self.len())
            .map_err(|_| Error::FileTooLarge)?;
        for index in 0..self.len() {
            let value = match self.value(index)? {
                PackedValue::Inline(value) => LeafValue::Inline(value.into()),
                PackedValue::Overflow {
                    head,
                    len,
                    checksum,
                } => LeafValue::Overflow {
                    head,
                    len,
                    checksum,
                },
            };
            entries.push(LeafEntry {
                key: self.key(index)?.into_boxed_slice(),
                value,
            });
        }
        Ok(entries)
    }

    pub(crate) fn overflow_heads(&self) -> impl Iterator<Item = PageId> + '_ {
        self.overflows.iter().map(|overflow| overflow.head)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.slots.is_empty() {
            return Err(Error::InternalInvariant("leaf must contain an entry"));
        }
        for index in 0..self.len() {
            let key_len = self
                .prefix()?
                .len()
                .checked_add(self.suffix(index)?.len())
                .ok_or(Error::InternalInvariant("leaf key length"))?;
            if key_len == 0 || key_len > MAX_KEY_BYTES {
                return Err(Error::InternalInvariant("leaf key length"));
            }
            match self.value(index)? {
                PackedValue::Inline(value) if value.len() <= MAX_INLINE_VALUE_BYTES => {}
                PackedValue::Overflow { len, .. }
                    if len > MAX_INLINE_VALUE_BYTES as u64 && len <= MAX_VALUE_BYTES as u64 => {}
                _ => return Err(Error::InternalInvariant("leaf value representation")),
            }
            if index > 0 && self.compare_keys(index - 1, index)? != Ordering::Less {
                return Err(Error::InternalInvariant(
                    "leaf keys must be strictly ordered",
                ));
            }
        }
        Ok(())
    }

    fn compare_keys(&self, left: usize, right: usize) -> Result<Ordering> {
        Ok(self.suffix(left)?.cmp(self.suffix(right)?))
    }

    fn slot(&self, index: usize) -> Result<LeafSlot> {
        self.slots
            .get(index)
            .copied()
            .ok_or(Error::InternalInvariant("packed leaf slot"))
    }
}

fn compact_offset(value: usize) -> Result<u16> {
    u16::try_from(value).map_err(|_| Error::Corrupt("packed leaf offset is too large".into()))
}

fn compact_length(value: usize, field: &str) -> Result<u16> {
    u16::try_from(value).map_err(|_| Error::Corrupt(format!("{field} is too long")))
}

fn packed_range(bytes: &[u8], start: u16, len: u16) -> Result<&[u8]> {
    let start = usize::from(start);
    let end = start
        .checked_add(usize::from(len))
        .ok_or(Error::InternalInvariant("packed leaf range"))?;
    bytes
        .get(start..end)
        .ok_or(Error::InternalInvariant("packed leaf range"))
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::{LeafSlot, PackedLeafBuilder, PackedValue};
    use crate::domain::PageId;

    #[test]
    fn compact_slots_preserve_inline_and_overflow_entries() {
        assert_eq!(size_of::<LeafSlot>(), 8);
        let mut builder = PackedLeafBuilder::new(b"prefix-", 2).unwrap();
        builder.push_inline(b"a", b"one").unwrap();
        builder
            .push_overflow(b"b", PageId::from_raw(7).unwrap(), 4096, 123)
            .unwrap();
        let leaf = builder.finish().unwrap();

        assert_eq!(leaf.key(0).unwrap(), b"prefix-a");
        assert_eq!(leaf.key(1).unwrap(), b"prefix-b");
        assert_eq!(leaf.find(b"prefix-a").unwrap(), Ok(0));
        assert_eq!(leaf.find(b"prefix-aa").unwrap(), Err(1));
        assert_eq!(leaf.find(b"pref").unwrap(), Err(0));
        assert_eq!(leaf.find(b"prefix-z").unwrap(), Err(2));
        assert!(matches!(
            leaf.value(0).unwrap(),
            PackedValue::Inline(b"one")
        ));
        assert!(matches!(
            leaf.value(1).unwrap(),
            PackedValue::Overflow { len: 4096, .. }
        ));
        leaf.validate().unwrap();
        assert_eq!(leaf.to_entries().unwrap().len(), 2);
    }
}
