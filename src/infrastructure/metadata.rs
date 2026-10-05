use super::checksum::crc32c;
use crate::domain::{Head, PageId};
use crate::{Error, Result, TransactionId};

pub(crate) const PAGE_SIZE: usize = 4096;
pub(crate) const METADATA_SLOTS: usize = 2;
pub(crate) const METADATA_REGION_SIZE: u64 = (PAGE_SIZE * METADATA_SLOTS) as u64;
const META_MAGIC: [u8; 8] = *b"WSLDB\0\0\0";
const HEADER_CHECKSUM_END: usize = 48;
const HEADER_END: usize = 52;

pub(crate) const fn slot_offset(transaction_id: TransactionId) -> u64 {
    (transaction_id.value() % METADATA_SLOTS as u64) * PAGE_SIZE as u64
}

pub(crate) fn encode(head: Head) -> [u8; PAGE_SIZE] {
    let mut page = [0_u8; PAGE_SIZE];
    page[0..8].copy_from_slice(&META_MAGIC);
    page[12..16].copy_from_slice(&(PAGE_SIZE as u32).to_le_bytes());
    page[16..24].copy_from_slice(&head.transaction_id().value().to_le_bytes());
    page[24..32].copy_from_slice(&head.root().map_or(0, PageId::value).to_le_bytes());
    page[32..40].copy_from_slice(&head.last_page().to_le_bytes());
    page[40..48].copy_from_slice(&head.entry_count().to_le_bytes());
    let checksum = crc32c(&page[..HEADER_CHECKSUM_END]);
    page[HEADER_CHECKSUM_END..HEADER_END].copy_from_slice(&checksum.to_le_bytes());
    page
}

pub(crate) fn decode(page: &[u8; PAGE_SIZE]) -> Result<Option<Head>> {
    if page[0..8] != META_MAGIC {
        return Ok(None);
    }

    if page[8..12] != [0; 4]
        || read_u32(&page[12..16])? != PAGE_SIZE as u32
        || read_u32(&page[HEADER_CHECKSUM_END..HEADER_END])? != crc32c(&page[..HEADER_CHECKSUM_END])
        || page[HEADER_END..].iter().any(|byte| *byte != 0)
    {
        return Ok(None);
    }

    let transaction_id = TransactionId::from_raw(read_u64(&page[16..24])?);
    let root_raw = read_u64(&page[24..32])?;
    let last_page = read_u64(&page[32..40])?;
    let entry_count = read_u64(&page[40..48])?;
    if last_page < 1 {
        return Ok(None);
    }
    let root = if root_raw == 0 {
        None
    } else {
        let Some(root) = PageId::from_raw(root_raw) else {
            return Ok(None);
        };
        if root.value() > last_page {
            return Ok(None);
        }
        Some(root)
    };
    if root.is_none() != (entry_count == 0) {
        return Ok(None);
    }

    Ok(Some(Head::new(
        transaction_id,
        root,
        last_page,
        entry_count,
    )))
}

fn read_u32(bytes: &[u8]) -> Result<u32> {
    bytes
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| Error::InternalInvariant("u32 metadata field width"))
}

fn read_u64(bytes: &[u8]) -> Result<u64> {
    bytes
        .try_into()
        .map(u64::from_le_bytes)
        .map_err(|_| Error::InternalInvariant("u64 metadata field width"))
}

#[cfg(test)]
mod tests {
    use super::{HEADER_CHECKSUM_END, PAGE_SIZE, decode, encode, slot_offset};
    use crate::TransactionId;
    use crate::domain::Head;

    #[test]
    fn round_trip_empty_head() {
        let head = Head::empty();
        let page = encode(head);
        assert_eq!(
            &page[..52],
            &[
                0x57, 0x53, 0x4c, 0x44, 0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9f, 0xdb, 0xd9, 0x7c,
            ]
        );
        assert_eq!(decode(&page).unwrap(), Some(head));
    }

    #[test]
    fn changed_header_is_rejected() {
        let mut page = encode(Head::empty());
        page[HEADER_CHECKSUM_END - 1] ^= 1;
        assert_eq!(decode(&page).unwrap(), None);
    }

    #[test]
    fn nonzero_reserved_metadata_is_rejected() {
        for offset in [8, HEADER_CHECKSUM_END + 4] {
            let mut page = encode(Head::empty());
            page[offset] = 1;
            assert_eq!(decode(&page).unwrap(), None);
        }
    }

    #[test]
    fn transaction_parity_selects_the_metadata_slot() {
        assert_eq!(slot_offset(TransactionId::from_raw(0)), 0);
        assert_eq!(slot_offset(TransactionId::from_raw(1)), PAGE_SIZE as u64);
        assert_eq!(slot_offset(TransactionId::from_raw(2)), 0);
    }
}
