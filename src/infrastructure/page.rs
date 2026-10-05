use super::checksum::crc32c;
use super::metadata::PAGE_SIZE;
use super::packed_leaf::{PackedLeaf, PackedLeafBuilder, PackedValue};
use crate::domain::{MAX_INLINE_VALUE_BYTES, MAX_KEY_BYTES, MAX_VALUE_BYTES, PageId};
use crate::{Error, Result, TransactionId};

const PAGE_MAGIC: [u8; 4] = *b"WSLP";
const HEADER_SIZE: usize = 32;
const PAYLOAD_SIZE: usize = PAGE_SIZE - HEADER_SIZE;
pub(crate) const OVERFLOW_DATA_BYTES: usize = PAYLOAD_SIZE - 8;
const KIND_LEAF: u8 = 1;
const KIND_BRANCH: u8 = 2;
const KIND_OVERFLOW: u8 = 3;
const VALUE_INLINE: u8 = 1;
const VALUE_OVERFLOW: u8 = 2;
const MIN_LEAF_ENTRY_BYTES: usize = 2 + 1 + 4;
const MIN_BRANCH_SEPARATOR_BYTES: usize = 2 + 1 + 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LeafValue {
    Inline(Box<[u8]>),
    Overflow {
        head: PageId,
        len: u64,
        checksum: u32,
    },
}

impl LeafValue {
    fn encoded_len(&self) -> usize {
        match self {
            Self::Inline(value) => 1 + 4 + value.len(),
            Self::Overflow { .. } => 1 + 8 + 8 + 4,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LeafEntry {
    pub(crate) key: Box<[u8]>,
    pub(crate) value: LeafValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Node {
    Leaf(Vec<LeafEntry>),
    PackedLeaf(PackedLeaf),
    Branch {
        children: Vec<PageId>,
        separators: Vec<Vec<u8>>,
    },
    Overflow {
        next: Option<PageId>,
        bytes: Vec<u8>,
    },
}

impl Node {
    pub(crate) fn leaf_entries_owned(&self) -> Result<Option<Vec<LeafEntry>>> {
        match self {
            Self::Leaf(entries) => Ok(Some(entries.clone())),
            Self::PackedLeaf(leaf) => leaf.to_entries().map(Some),
            Self::Branch { .. } | Self::Overflow { .. } => Ok(None),
        }
    }

    pub(crate) fn leaf_encoded_len(entries: &[LeafEntry]) -> usize {
        let prefix_len = common_key_prefix_len(entries);
        4 + prefix_len
            + entries
                .iter()
                .map(|entry| 2 + entry.key.len() - prefix_len + entry.value.encoded_len())
                .sum::<usize>()
    }

    pub(crate) fn leaf_entries_fit(entries: &[LeafEntry]) -> bool {
        Self::leaf_encoded_len(entries) <= PAYLOAD_SIZE
    }

    pub(crate) fn branch_encoded_len(children: &[PageId], separators: &[Vec<u8>]) -> usize {
        let first_child = usize::from(!children.is_empty()) * 8;
        2 + first_child
            + separators
                .iter()
                .map(|separator| 10 + separator.len())
                .sum::<usize>()
    }

    pub(crate) fn branch_entries_fit(children: &[PageId], separators: &[Vec<u8>]) -> bool {
        Self::branch_encoded_len(children, separators) <= PAYLOAD_SIZE
    }

    pub(crate) fn encoded_len(&self) -> usize {
        match self {
            Self::Leaf(entries) => Self::leaf_encoded_len(entries),
            Self::PackedLeaf(leaf) => leaf.encoded_len(),
            Self::Branch {
                children,
                separators,
            } => Self::branch_encoded_len(children, separators),
            Self::Overflow { bytes, .. } => 8 + bytes.len(),
        }
    }

    pub(crate) fn fits(&self) -> bool {
        self.encoded_len() <= PAYLOAD_SIZE
    }

    pub(crate) fn encode(
        &self,
        page_id: PageId,
        transaction_id: TransactionId,
    ) -> Result<[u8; PAGE_SIZE]> {
        validate_node(self)?;
        if !self.fits() {
            return Err(Error::InternalInvariant(
                "attempted to write an oversized page",
            ));
        }

        let mut payload = Vec::with_capacity(self.encoded_len());
        let kind = match self {
            Self::Leaf(entries) => {
                let prefix_len = common_key_prefix_len(entries);
                push_u16(&mut payload, entries.len())?;
                push_u16(&mut payload, prefix_len)?;
                payload.extend_from_slice(&entries[0].key[..prefix_len]);
                for entry in entries {
                    let suffix = &entry.key[prefix_len..];
                    push_u16(&mut payload, suffix.len())?;
                    payload.extend_from_slice(suffix);
                    encode_value(&mut payload, &entry.value)?;
                }
                KIND_LEAF
            }
            Self::PackedLeaf(leaf) => {
                push_u16(&mut payload, leaf.len())?;
                let prefix = leaf.prefix()?;
                push_u16(&mut payload, prefix.len())?;
                payload.extend_from_slice(prefix);
                for index in 0..leaf.len() {
                    let suffix = leaf.suffix(index)?;
                    push_u16(&mut payload, suffix.len())?;
                    payload.extend_from_slice(suffix);
                    match leaf.value(index)? {
                        PackedValue::Inline(value) => {
                            payload.push(VALUE_INLINE);
                            push_u32(&mut payload, value.len())?;
                            payload.extend_from_slice(value);
                        }
                        PackedValue::Overflow {
                            head,
                            len,
                            checksum,
                        } => {
                            payload.push(VALUE_OVERFLOW);
                            payload.extend_from_slice(&head.value().to_le_bytes());
                            payload.extend_from_slice(&len.to_le_bytes());
                            payload.extend_from_slice(&checksum.to_le_bytes());
                        }
                    }
                }
                KIND_LEAF
            }
            Self::Branch {
                children,
                separators,
            } => {
                push_u16(&mut payload, children.len())?;
                payload.extend_from_slice(&children[0].value().to_le_bytes());
                for (separator, child) in separators.iter().zip(&children[1..]) {
                    push_u16(&mut payload, separator.len())?;
                    payload.extend_from_slice(separator);
                    payload.extend_from_slice(&child.value().to_le_bytes());
                }
                KIND_BRANCH
            }
            Self::Overflow { next, bytes } => {
                payload.extend_from_slice(&next.map_or(0, PageId::value).to_le_bytes());
                payload.extend_from_slice(bytes);
                KIND_OVERFLOW
            }
        };

        let mut page = [0_u8; PAGE_SIZE];
        page[0..4].copy_from_slice(&PAGE_MAGIC);
        page[5] = kind;
        page[8..12].copy_from_slice(
            &u32::try_from(payload.len())
                .map_err(|_| Error::InternalInvariant("page payload length"))?
                .to_le_bytes(),
        );
        page[12..20].copy_from_slice(&transaction_id.value().to_le_bytes());
        page[20..24].copy_from_slice(&crc32c(&payload).to_le_bytes());
        let header_checksum = header_checksum(&page, page_id);
        page[24..28].copy_from_slice(&header_checksum.to_le_bytes());
        page[HEADER_SIZE..HEADER_SIZE + payload.len()].copy_from_slice(&payload);
        Ok(page)
    }

    pub(crate) fn decode(
        page: &[u8; PAGE_SIZE],
        page_id: PageId,
        maximum_transaction_id: TransactionId,
    ) -> Result<Self> {
        if page[0..4] != PAGE_MAGIC
            || page[4] != 0
            || page[6..8] != [0; 2]
            || page[28..HEADER_SIZE] != [0; 4]
        {
            return Err(Error::Corrupt("page header is not recognized".into()));
        }
        if read_u32(&page[24..28])? != header_checksum(page, page_id) {
            return Err(Error::Corrupt("page header checksum does not match".into()));
        }
        let payload_len = usize::try_from(read_u32(&page[8..12])?)
            .map_err(|_| Error::Corrupt("page payload length does not fit in memory".into()))?;
        if payload_len > PAYLOAD_SIZE {
            return Err(Error::Corrupt(
                "page payload exceeds the page boundary".into(),
            ));
        }
        let page_transaction_id = TransactionId::from_raw(read_u64(&page[12..20])?);
        if page_transaction_id.value() == 0 || page_transaction_id > maximum_transaction_id {
            return Err(Error::Corrupt("page transaction ID is not visible".into()));
        }
        let payload = &page[HEADER_SIZE..HEADER_SIZE + payload_len];
        if read_u32(&page[20..24])? != crc32c(payload) {
            return Err(Error::Corrupt(
                "page payload checksum does not match".into(),
            ));
        }
        if page[HEADER_SIZE + payload_len..]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(Error::Corrupt("page padding is not zero".into()));
        }

        let mut input = Input::new(payload);
        let node = match page[5] {
            KIND_LEAF => decode_leaf(&mut input, page_id)?,
            KIND_BRANCH => decode_branch(&mut input, page_id)?,
            KIND_OVERFLOW => decode_overflow(&mut input, page_id)?,
            _ => return Err(Error::Corrupt("page kind is unsupported".into())),
        };
        if !input.is_empty() {
            return Err(Error::Corrupt("page payload has trailing bytes".into()));
        }
        validate_node(&node).map_err(|error| Error::Corrupt(error.to_string()))?;
        Ok(node)
    }
}

fn common_key_prefix_len(entries: &[LeafEntry]) -> usize {
    let Some(first) = entries.first() else {
        return 0;
    };
    entries
        .iter()
        .skip(1)
        .fold(first.key.len(), |length, entry| {
            first.key[..length]
                .iter()
                .zip(&entry.key)
                .take_while(|(left, right)| left == right)
                .count()
        })
}

fn header_checksum(page: &[u8; PAGE_SIZE], page_id: PageId) -> u32 {
    let mut protected = [0_u8; 32];
    protected[..24].copy_from_slice(&page[..24]);
    protected[24..].copy_from_slice(&page_id.value().to_le_bytes());
    crc32c(&protected)
}

fn encode_value(bytes: &mut Vec<u8>, value: &LeafValue) -> Result<()> {
    match value {
        LeafValue::Inline(value) => {
            bytes.push(VALUE_INLINE);
            push_u32(bytes, value.len())?;
            bytes.extend_from_slice(value);
        }
        LeafValue::Overflow {
            head,
            len,
            checksum,
        } => {
            bytes.push(VALUE_OVERFLOW);
            bytes.extend_from_slice(&head.value().to_le_bytes());
            bytes.extend_from_slice(&len.to_le_bytes());
            bytes.extend_from_slice(&checksum.to_le_bytes());
        }
    }
    Ok(())
}

fn decode_leaf(input: &mut Input<'_>, page_id: PageId) -> Result<Node> {
    let count = usize::from(input.read_u16()?);
    if count == 0 {
        return Err(Error::Corrupt("leaf page is empty".into()));
    }
    let prefix_len = usize::from(input.read_u16()?);
    if prefix_len > MAX_KEY_BYTES {
        return Err(Error::Corrupt("leaf key prefix is too long".into()));
    }
    let prefix = input.take(prefix_len)?;
    if count > input.remaining() / MIN_LEAF_ENTRY_BYTES {
        return Err(Error::Corrupt(
            "leaf entry count exceeds its payload".into(),
        ));
    }
    let mut entries = PackedLeafBuilder::new(prefix, count)?;
    for _ in 0..count {
        let suffix_len = usize::from(input.read_u16()?);
        let key_len = prefix_len
            .checked_add(suffix_len)
            .ok_or_else(|| Error::Corrupt("leaf key length overflow".into()))?;
        if key_len > MAX_KEY_BYTES {
            return Err(Error::Corrupt("leaf key is too long".into()));
        }
        let suffix = input.take(suffix_len)?;
        match input.read_u8()? {
            VALUE_INLINE => {
                let len = usize::try_from(input.read_u32()?)
                    .map_err(|_| Error::Corrupt("value length does not fit in memory".into()))?;
                entries.push_inline(suffix, input.take(len)?)?;
            }
            VALUE_OVERFLOW => {
                let head = decode_child(input.read_u64()?, page_id, "overflow head")?;
                entries.push_overflow(suffix, head, input.read_u64()?, input.read_u32()?)?;
            }
            _ => return Err(Error::Corrupt("leaf value kind is unsupported".into())),
        }
    }
    Ok(Node::PackedLeaf(entries.finish()?))
}

fn decode_branch(input: &mut Input<'_>, page_id: PageId) -> Result<Node> {
    let child_count = usize::from(input.read_u16()?);
    if child_count < 2 {
        return Err(Error::Corrupt("branch has fewer than two children".into()));
    }
    let first_child = decode_child(input.read_u64()?, page_id, "branch child")?;
    let separator_count = child_count - 1;
    if separator_count > input.remaining() / MIN_BRANCH_SEPARATOR_BYTES {
        return Err(Error::Corrupt(
            "branch child count exceeds its payload".into(),
        ));
    }
    let mut children = Vec::new();
    children
        .try_reserve_exact(child_count)
        .map_err(|_| Error::FileTooLarge)?;
    let mut separators = Vec::new();
    separators
        .try_reserve_exact(separator_count)
        .map_err(|_| Error::FileTooLarge)?;
    children.push(first_child);
    for _ in 1..child_count {
        let key_len = usize::from(input.read_u16()?);
        if key_len == 0 || key_len > MAX_KEY_BYTES {
            return Err(Error::Corrupt("branch separator length is invalid".into()));
        }
        separators.push(input.take(key_len)?.to_vec());
        children.push(decode_child(input.read_u64()?, page_id, "branch child")?);
    }
    Ok(Node::Branch {
        children,
        separators,
    })
}

fn decode_overflow(input: &mut Input<'_>, page_id: PageId) -> Result<Node> {
    let next_raw = input.read_u64()?;
    let next = if next_raw == 0 {
        None
    } else {
        Some(decode_child(next_raw, page_id, "overflow continuation")?)
    };
    let bytes = input.take(input.remaining())?.to_vec();
    Ok(Node::Overflow { next, bytes })
}

fn decode_child(raw: u64, parent: PageId, role: &str) -> Result<PageId> {
    let child = PageId::from_raw(raw)
        .ok_or_else(|| Error::Corrupt(format!("{role} page ID is invalid")))?;
    if child >= parent {
        return Err(Error::Corrupt(format!(
            "{role} does not precede its immutable parent"
        )));
    }
    Ok(child)
}

fn validate_node(node: &Node) -> Result<()> {
    match node {
        Node::Leaf(entries) => {
            if entries.is_empty() {
                return Err(Error::InternalInvariant("leaf must contain an entry"));
            }
            for entry in entries {
                if entry.key.is_empty() || entry.key.len() > MAX_KEY_BYTES {
                    return Err(Error::InternalInvariant("leaf key length"));
                }
                match &entry.value {
                    LeafValue::Inline(value) if value.len() <= MAX_INLINE_VALUE_BYTES => {}
                    LeafValue::Overflow { len, .. }
                        if *len > MAX_INLINE_VALUE_BYTES as u64
                            && *len <= MAX_VALUE_BYTES as u64 => {}
                    _ => return Err(Error::InternalInvariant("leaf value representation")),
                }
            }
            if entries.windows(2).any(|pair| pair[0].key >= pair[1].key) {
                return Err(Error::InternalInvariant(
                    "leaf keys must be strictly ordered",
                ));
            }
        }
        Node::PackedLeaf(leaf) => leaf.validate()?,
        Node::Branch {
            children,
            separators,
        } => {
            if children.len() < 2 || separators.len() + 1 != children.len() {
                return Err(Error::InternalInvariant("branch arity"));
            }
            if separators
                .iter()
                .any(|key| key.is_empty() || key.len() > MAX_KEY_BYTES)
            {
                return Err(Error::InternalInvariant("branch separator length"));
            }
            if separators.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(Error::InternalInvariant(
                    "branch separators must be strictly ordered",
                ));
            }
        }
        Node::Overflow { bytes, .. } => {
            if bytes.is_empty() || bytes.len() > OVERFLOW_DATA_BYTES {
                return Err(Error::InternalInvariant("overflow payload length"));
            }
        }
    }
    Ok(())
}

fn push_u16(bytes: &mut Vec<u8>, value: usize) -> Result<()> {
    bytes.extend_from_slice(
        &u16::try_from(value)
            .map_err(|_| Error::InternalInvariant("u16 page field overflow"))?
            .to_le_bytes(),
    );
    Ok(())
}

fn push_u32(bytes: &mut Vec<u8>, value: usize) -> Result<()> {
    bytes.extend_from_slice(
        &u32::try_from(value)
            .map_err(|_| Error::InternalInvariant("u32 page field overflow"))?
            .to_le_bytes(),
    );
    Ok(())
}

fn read_u32(bytes: &[u8]) -> Result<u32> {
    bytes
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| Error::InternalInvariant("u32 page field width"))
}

fn read_u64(bytes: &[u8]) -> Result<u64> {
    bytes
        .try_into()
        .map(u64::from_le_bytes)
        .map_err(|_| Error::InternalInvariant("u64 page field width"))
}

struct Input<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Input<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(len)
            .ok_or_else(|| Error::Corrupt("page length overflow".into()))?;
        let result = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| Error::Corrupt("page payload is truncated".into()))?;
        self.position = end;
        Ok(result)
    }

    fn read_u8(&mut self) -> Result<u8> {
        self.take(1)?
            .first()
            .copied()
            .ok_or_else(|| Error::Corrupt("u8 page field is missing".into()))
    }

    fn read_u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take_array()?))
    }

    fn read_u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take_array()?))
    }

    fn read_u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take_array()?))
    }

    fn take_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| Error::Corrupt("page field has the wrong width".into()))
    }

    const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    const fn is_empty(&self) -> bool {
        self.position == self.bytes.len()
    }
}

#[cfg(test)]
mod tests;
