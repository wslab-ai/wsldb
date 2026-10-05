use super::checksum::crc32c;
use super::page::{LeafValue, Node, OVERFLOW_DATA_BYTES};
use super::tree::{PageReader, PageWriter};
use crate::domain::MAX_VALUE_BYTES;
use crate::{Error, Result};

pub(crate) fn store_value(writer: &mut impl PageWriter, value: &[u8]) -> Result<LeafValue> {
    if value.len() <= crate::domain::MAX_INLINE_VALUE_BYTES {
        return Ok(LeafValue::Inline(value.into()));
    }

    let mut next = None;
    for chunk in value.rchunks(OVERFLOW_DATA_BYTES) {
        next = Some(writer.write_node(&Node::Overflow {
            next,
            bytes: chunk.to_vec(),
        })?);
    }
    let head = next.ok_or(Error::InternalInvariant("missing overflow head"))?;
    Ok(LeafValue::Overflow {
        head,
        len: u64::try_from(value.len()).map_err(|_| Error::FileTooLarge)?,
        checksum: crc32c(value),
    })
}

pub(crate) fn read_value(reader: &impl PageReader, value: &LeafValue) -> Result<Vec<u8>> {
    let LeafValue::Overflow {
        head,
        len,
        checksum,
    } = value
    else {
        let LeafValue::Inline(value) = value else {
            return Err(Error::InternalInvariant("leaf value variant"));
        };
        return Ok(value.to_vec());
    };

    let expected_len = usize::try_from(*len)
        .map_err(|_| Error::Corrupt("overflow value length does not fit in memory".into()))?;
    if expected_len > MAX_VALUE_BYTES {
        return Err(Error::Corrupt(
            "overflow value exceeds the configured limit".into(),
        ));
    }
    let maximum_pages = expected_len.div_ceil(OVERFLOW_DATA_BYTES);
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(expected_len)
        .map_err(|_| Error::FileTooLarge)?;
    let mut current = Some(*head);
    for _ in 0..maximum_pages {
        let page_id = current.ok_or_else(|| Error::Corrupt("overflow chain ended early".into()))?;
        match reader.read_node(page_id)?.as_ref() {
            Node::Overflow { next, bytes: chunk } => {
                bytes.extend_from_slice(chunk);
                current = *next;
            }
            Node::Leaf(_) | Node::PackedLeaf(_) | Node::Branch { .. } => {
                return Err(Error::Corrupt(
                    "overflow chain references a tree page".into(),
                ));
            }
        }
    }
    if current.is_some() || bytes.len() != expected_len || crc32c(&bytes) != *checksum {
        return Err(Error::Corrupt("overflow value validation failed".into()));
    }
    Ok(bytes)
}
