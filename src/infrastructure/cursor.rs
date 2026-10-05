use super::packed_leaf::{PackedLeaf, PackedValue};
use super::page::Node;
use super::tree::{PageReader, read_value};
use crate::domain::{Head, MAX_TREE_DEPTH, PageId};
use crate::{Error, Result};

pub(crate) fn scan(reader: &impl PageReader, head: Head) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let Some(root) = head.root() else {
        return Ok(Vec::new());
    };
    let capacity = usize::try_from(head.entry_count())
        .map_err(|_| Error::Corrupt("entry count does not fit in memory".into()))?;
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(capacity)
        .map_err(|_| Error::FileTooLarge)?;
    scan_page(reader, root, 0, capacity, &mut entries)?;

    if entries.len() != capacity {
        return Err(Error::Corrupt(
            "tree entry count does not match metadata".into(),
        ));
    }
    if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(Error::Corrupt("tree keys are not strictly ordered".into()));
    }
    Ok(entries)
}

pub(crate) fn range(
    reader: &impl PageReader,
    head: Head,
    start: &[u8],
    start_exclusive: bool,
    end: Option<&[u8]>,
    limit: usize,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    if limit == 0 || end.is_some_and(|end| end <= start) {
        return Ok(Vec::new());
    }
    let Some(root) = head.root() else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    entries
        .try_reserve(limit.min(1024))
        .map_err(|_| Error::FileTooLarge)?;
    let bounds = RangeBounds {
        start,
        start_exclusive,
        end,
    };
    range_page(reader, root, 0, bounds, limit, &mut entries)?;
    Ok(entries)
}

#[derive(Clone, Copy)]
struct RangeBounds<'a> {
    start: &'a [u8],
    start_exclusive: bool,
    end: Option<&'a [u8]>,
}

fn scan_page(
    reader: &impl PageReader,
    page_id: PageId,
    depth: usize,
    maximum_entries: usize,
    output: &mut Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<()> {
    ensure_depth(depth)?;
    match reader.read_node(page_id)?.as_ref() {
        Node::Leaf(entries) => {
            for entry in entries {
                reject_excess_entry(output.len(), maximum_entries)?;
                output.push((entry.key.to_vec(), read_value(reader, &entry.value)?));
            }
        }
        Node::PackedLeaf(leaf) => {
            for index in 0..leaf.len() {
                reject_excess_entry(output.len(), maximum_entries)?;
                output.push((leaf.key(index)?, read_packed_value(reader, leaf, index)?));
            }
        }
        Node::Branch { children, .. } => {
            for child in children {
                scan_page(reader, *child, depth + 1, maximum_entries, output)?;
            }
        }
        Node::Overflow { .. } => {
            return Err(Error::Corrupt(
                "tree contains an overflow page as a node".into(),
            ));
        }
    }
    Ok(())
}

fn range_page(
    reader: &impl PageReader,
    page_id: PageId,
    depth: usize,
    bounds: RangeBounds<'_>,
    limit: usize,
    output: &mut Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<()> {
    let RangeBounds {
        start,
        start_exclusive,
        end,
    } = bounds;
    ensure_depth(depth)?;
    match reader.read_node(page_id)?.as_ref() {
        Node::Leaf(entries) => {
            let first = entries.partition_point(|entry| {
                if start_exclusive {
                    entry.key.as_ref() <= start
                } else {
                    entry.key.as_ref() < start
                }
            });
            for entry in &entries[first..] {
                if output.len() == limit || end.is_some_and(|end| entry.key.as_ref() >= end) {
                    break;
                }
                output.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
                output.push((entry.key.to_vec(), read_value(reader, &entry.value)?));
            }
        }
        Node::PackedLeaf(leaf) => {
            let first = match leaf.find(start)? {
                Ok(index) if start_exclusive => index + 1,
                Ok(index) | Err(index) => index,
            };
            for index in first..leaf.len() {
                if output.len() == limit {
                    break;
                }
                let key = leaf.key(index)?;
                if end.is_some_and(|end| key.as_slice() >= end) {
                    break;
                }
                output.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
                output.push((key, read_packed_value(reader, leaf, index)?));
            }
        }
        Node::Branch {
            children,
            separators,
        } => {
            let first = separators.partition_point(|separator| separator.as_slice() <= start);
            for index in first..children.len() {
                if output.len() == limit {
                    break;
                }
                if index > 0 && end.is_some_and(|end| separators[index - 1].as_slice() >= end) {
                    break;
                }
                let child_start = if index == first { start } else { &[] };
                let child_start_exclusive = index == first && start_exclusive;
                range_page(
                    reader,
                    children[index],
                    depth + 1,
                    RangeBounds {
                        start: child_start,
                        start_exclusive: child_start_exclusive,
                        end,
                    },
                    limit,
                    output,
                )?;
            }
        }
        Node::Overflow { .. } => {
            return Err(Error::Corrupt(
                "tree contains an overflow page as a node".into(),
            ));
        }
    }
    Ok(())
}

fn reject_excess_entry(actual: usize, expected: usize) -> Result<()> {
    if actual >= expected {
        Err(Error::Corrupt(
            "tree contains more entries than its metadata declares".into(),
        ))
    } else {
        Ok(())
    }
}

fn ensure_depth(depth: usize) -> Result<()> {
    if depth >= MAX_TREE_DEPTH {
        Err(Error::TreeDepthExceeded)
    } else {
        Ok(())
    }
}

fn read_packed_value(reader: &impl PageReader, leaf: &PackedLeaf, index: usize) -> Result<Vec<u8>> {
    match leaf.value(index)? {
        PackedValue::Inline(value) => Ok(value.to_vec()),
        PackedValue::Overflow {
            head,
            len,
            checksum,
        } => read_value(
            reader,
            &super::page::LeafValue::Overflow {
                head,
                len,
                checksum,
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::{PageReader, range, scan};
    use crate::domain::{Head, PageId};
    use crate::infrastructure::page::{LeafEntry, LeafValue, Node};
    use crate::{Error, Result, TransactionId};

    #[test]
    fn scan_rejects_excess_entries_before_following_more_pages() {
        let root = page_id(4);
        let reader = MemoryReader::new([
            (
                root,
                Node::Branch {
                    children: vec![page_id(2), page_id(3)],
                    separators: vec![b"z".to_vec()],
                },
            ),
            (page_id(2), leaf(&[(b"a", b"one"), (b"b", b"two")])),
        ]);
        let head = Head::new(TransactionId::from_raw(1), Some(root), 4, 1);

        assert!(
            matches!(scan(&reader, head), Err(Error::Corrupt(message)) if
            message == "tree contains more entries than its metadata declares")
        );
    }

    #[test]
    fn bounded_range_preserves_order_and_limit() {
        let root = page_id(4);
        let reader = MemoryReader::new([
            (
                root,
                Node::Branch {
                    children: vec![page_id(2), page_id(3)],
                    separators: vec![b"c".to_vec()],
                },
            ),
            (page_id(2), leaf(&[(b"a", b"one"), (b"b", b"two")])),
            (page_id(3), leaf(&[(b"c", b"three"), (b"d", b"four")])),
        ]);
        let head = Head::new(TransactionId::from_raw(1), Some(root), 4, 4);

        assert_eq!(
            range(&reader, head, b"b", false, None, 2).unwrap(),
            vec![
                (b"b".to_vec(), b"two".to_vec()),
                (b"c".to_vec(), b"three".to_vec())
            ]
        );
        assert_eq!(
            range(&reader, head, b"b", true, None, 2).unwrap(),
            vec![
                (b"c".to_vec(), b"three".to_vec()),
                (b"d".to_vec(), b"four".to_vec())
            ]
        );
    }

    #[test]
    fn exclusive_start_does_not_read_the_boundary_value() {
        let root = page_id(4);
        let reader = MemoryReader::new([(
            root,
            Node::Leaf(vec![
                LeafEntry {
                    key: b"b".as_slice().into(),
                    value: LeafValue::Overflow {
                        head: page_id(2),
                        len: 3_000,
                        checksum: 0,
                    },
                },
                LeafEntry {
                    key: b"c".as_slice().into(),
                    value: LeafValue::Inline(b"three".as_slice().into()),
                },
            ]),
        )]);
        let head = Head::new(TransactionId::from_raw(1), Some(root), 4, 2);

        assert_eq!(
            range(&reader, head, b"b", true, None, 1).unwrap(),
            vec![(b"c".to_vec(), b"three".to_vec())]
        );
    }

    fn page_id(raw: u64) -> PageId {
        PageId::from_raw(raw).unwrap()
    }

    fn leaf(entries: &[(&[u8], &[u8])]) -> Node {
        Node::Leaf(
            entries
                .iter()
                .map(|(key, value)| LeafEntry {
                    key: (*key).into(),
                    value: LeafValue::Inline((*value).into()),
                })
                .collect(),
        )
    }

    struct MemoryReader {
        nodes: BTreeMap<PageId, Arc<Node>>,
    }

    impl MemoryReader {
        fn new(nodes: impl IntoIterator<Item = (PageId, Node)>) -> Self {
            Self {
                nodes: nodes
                    .into_iter()
                    .map(|(page_id, node)| (page_id, Arc::new(node)))
                    .collect(),
            }
        }
    }

    impl PageReader for MemoryReader {
        fn read_node(&self, page_id: PageId) -> Result<Arc<Node>> {
            self.nodes
                .get(&page_id)
                .cloned()
                .ok_or(Error::InternalInvariant("test page is missing"))
        }
    }
}
