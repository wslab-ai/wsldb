use crate::{Error, Result};

const BITS_PER_ENTRY: u64 = 10;
const HASH_PROBES: u64 = 7;
const MINIMUM_BITS: u64 = 64;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const INITIAL_GROWING_ENTRIES: u64 = 64 * 1024;

/// In-memory negative index for keys in an immutable checkpoint.
///
/// Positive answers are deliberately inconclusive. Callers must consult the
/// tree before treating a key as present.
#[derive(Clone)]
pub(crate) struct MembershipFilter {
    words: Vec<u64>,
    bit_count: u64,
}

/// A Bloom filter that reserves memory in proportion to observed keys.
///
/// Its exact backing map permits infrequent geometric rebuilds before WAL
/// synchronization. Inserts after durable publication never allocate.
#[derive(Clone)]
pub(crate) struct GrowingMembershipFilter {
    filter: MembershipFilter,
    capacity: u64,
}

impl GrowingMembershipFilter {
    pub(crate) fn new(estimated_maximum_entries: u64) -> Result<Self> {
        let capacity = estimated_maximum_entries.clamp(1, INITIAL_GROWING_ENTRIES);
        Ok(Self {
            filter: MembershipFilter::for_entries(capacity)?,
            capacity,
        })
    }

    pub(crate) const fn capacity(&self) -> u64 {
        self.capacity
    }

    pub(crate) fn ensure_capacity<'a>(
        &mut self,
        required: u64,
        keys: impl IntoIterator<Item = &'a [u8]>,
    ) -> Result<()> {
        if required <= self.capacity {
            return Ok(());
        }
        let mut capacity = self.capacity;
        while capacity < required {
            capacity = capacity.checked_mul(2).ok_or(Error::FileTooLarge)?;
        }
        let mut filter = MembershipFilter::for_entries(capacity)?;
        for key in keys {
            filter.insert(key);
        }
        self.filter = filter;
        self.capacity = capacity;
        Ok(())
    }

    pub(crate) fn insert(&mut self, key: &[u8]) {
        self.filter.insert(key);
    }

    pub(crate) fn may_contain(&self, key: &[u8]) -> bool {
        self.filter.may_contain(key)
    }
}

impl MembershipFilter {
    pub(crate) fn for_entries(entries: u64) -> Result<Self> {
        let requested = entries
            .checked_mul(BITS_PER_ENTRY)
            .ok_or(Error::FileTooLarge)?
            .max(MINIMUM_BITS);
        let word_count = requested.checked_add(63).ok_or(Error::FileTooLarge)? / 64;
        let bit_count = word_count.checked_mul(64).ok_or(Error::FileTooLarge)?;
        let word_count = usize::try_from(word_count).map_err(|_| Error::FileTooLarge)?;
        let mut words = Vec::new();
        words
            .try_reserve_exact(word_count)
            .map_err(|_| Error::FileTooLarge)?;
        words.resize(word_count, 0);
        Ok(Self { words, bit_count })
    }

    pub(crate) fn insert(&mut self, key: &[u8]) {
        Self::visit_positions(self.bit_count, key, |word, mask| self.words[word] |= mask);
    }

    pub(crate) fn may_contain(&self, key: &[u8]) -> bool {
        let mut present = true;
        Self::visit_positions(self.bit_count, key, |word, mask| {
            present &= self.words[word] & mask != 0;
        });
        present
    }

    fn visit_positions(bit_count: u64, key: &[u8], mut visit: impl FnMut(usize, u64)) {
        let first = mix64(fnv1a(key));
        let step = mix64(first ^ 0x9e37_79b9_7f4a_7c15) | 1;
        for probe in 0..HASH_PROBES {
            let bit = first.wrapping_add(probe.wrapping_mul(step)) % bit_count;
            visit((bit / 64) as usize, 1_u64 << (bit % 64));
        }
    }
}

fn fnv1a(key: &[u8]) -> u64 {
    key.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

const fn mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::{GrowingMembershipFilter, MembershipFilter};

    #[test]
    fn inserted_keys_never_report_absent() {
        let mut filter = MembershipFilter::for_entries(10_000).unwrap();
        for value in 0_u64..10_000 {
            filter.insert(&value.to_le_bytes());
        }
        for value in 0_u64..10_000 {
            assert!(filter.may_contain(&value.to_le_bytes()));
        }
    }

    #[test]
    fn false_positive_rate_stays_bounded_for_deterministic_keys() {
        let mut filter = MembershipFilter::for_entries(10_000).unwrap();
        for value in 0_u64..10_000 {
            filter.insert(&value.to_le_bytes());
        }
        let false_positives = (10_000_u64..20_000)
            .filter(|value| filter.may_contain(&value.to_le_bytes()))
            .count();
        assert!(false_positives <= 200, "{false_positives} false positives");
    }

    #[test]
    fn growing_filter_rebuilds_without_false_negatives() {
        let mut filter = GrowingMembershipFilter::new(1).unwrap();
        let keys = (0_u64..130_000)
            .map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        filter
            .ensure_capacity(keys.len() as u64, keys.iter().map(<[u8; 8]>::as_slice))
            .unwrap();
        for key in &keys {
            filter.insert(key);
        }
        assert!(filter.capacity() >= keys.len() as u64);
        for key in &keys {
            assert!(filter.may_contain(key));
        }
    }
}
