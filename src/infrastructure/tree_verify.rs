use std::collections::HashSet;

use super::checksum::crc32c;
use super::page::{LeafEntry, LeafValue, Node, OVERFLOW_DATA_BYTES};
use super::sha256::Sha256;
use super::tree::PageReader;
use crate::domain::{Head, MAX_TREE_DEPTH, PageId};
use crate::{Error, Result, VerificationReport};

pub(crate) fn verify(reader: &impl PageReader, head: Head) -> Result<VerificationReport> {
    let mut logical = LogicalSnapshot::new();
    let tree = verify_with_entries(reader, head, |key, value| logical.push(key, &value))?;
    logical.finish(head, tree)
}

pub(crate) fn verify_materialized(
    head: Head,
    entries: &[(Vec<u8>, Vec<u8>)],
    physical: VerificationReport,
) -> Result<VerificationReport> {
    if entries
        .windows(2)
        .any(|pair| pair[0].0.as_slice() >= pair[1].0.as_slice())
    {
        return Err(Error::Corrupt(
            "materialized snapshot keys are not strictly ordered".into(),
        ));
    }
    let mut logical = LogicalSnapshot::new();
    for (key, value) in entries {
        logical.push(key, value)?;
    }
    logical.finish(
        head,
        VerifiedTree {
            tree_pages: physical.tree_pages,
            overflow_pages: physical.overflow_pages,
            tree_depth: physical.tree_depth,
        },
    )
}

pub(crate) fn verify_with_entries(
    reader: &impl PageReader,
    head: Head,
    mut visitor: impl FnMut(&[u8], Vec<u8>) -> Result<()>,
) -> Result<VerifiedTree> {
    let Some(root) = head.root() else {
        if head.entry_count() != 0 {
            return Err(Error::Corrupt(
                "empty root has a non-zero entry count".into(),
            ));
        }
        return Ok(VerifiedTree {
            tree_pages: 0,
            overflow_pages: 0,
            tree_depth: 0,
        });
    };

    let mut traversal = Traversal {
        reader,
        visited: HashSet::new(),
        overflow_pages: 0,
        verified_entries: 0,
        maximum_entries: head.entry_count(),
        visitor: &mut visitor,
    };
    let summary = traversal.verify_page(root, 0)?;
    if summary.entries != head.entry_count() {
        return Err(Error::Corrupt(
            "tree entry count does not match metadata".into(),
        ));
    }
    let total_pages = u64::try_from(traversal.visited.len()).map_err(|_| Error::FileTooLarge)?;
    let tree_pages = total_pages
        .checked_sub(traversal.overflow_pages)
        .ok_or(Error::InternalInvariant("verification page count"))?;
    Ok(VerifiedTree {
        tree_pages,
        overflow_pages: traversal.overflow_pages,
        tree_depth: summary.depth,
    })
}

pub(crate) struct LogicalSnapshot {
    hash: Sha256,
    entries: u64,
}

impl LogicalSnapshot {
    pub(crate) fn new() -> Self {
        let mut hash = Sha256::new();
        hash.update(b"WSLDB logical snapshot\0");
        Self { hash, entries: 0 }
    }

    pub(crate) fn push(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        self.hash.update(
            &u64::try_from(key.len())
                .map_err(|_| Error::FileTooLarge)?
                .to_le_bytes(),
        );
        self.hash.update(key);
        self.hash.update(
            &u64::try_from(value.len())
                .map_err(|_| Error::FileTooLarge)?
                .to_le_bytes(),
        );
        self.hash.update(value);
        self.entries = self.entries.checked_add(1).ok_or(Error::FileTooLarge)?;
        Ok(())
    }

    pub(crate) fn finish(self, head: Head, tree: VerifiedTree) -> Result<VerificationReport> {
        let (entries, logical_hash) = self.finish_summary();
        if entries != head.entry_count() {
            return Err(Error::Corrupt(
                "logical snapshot entry count does not match metadata".into(),
            ));
        }
        Ok(VerificationReport {
            transaction_id: head.transaction_id(),
            entries,
            tree_pages: tree.tree_pages,
            overflow_pages: tree.overflow_pages,
            tree_depth: tree.tree_depth,
            logical_hash,
        })
    }

    pub(crate) fn finish_summary(self) -> (u64, [u8; 32]) {
        (self.entries, self.hash.finalize())
    }
}

pub(crate) struct VerifiedTree {
    tree_pages: u64,
    overflow_pages: u64,
    tree_depth: u32,
}

struct SubtreeSummary {
    minimum: Vec<u8>,
    maximum: Vec<u8>,
    entries: u64,
    depth: u32,
}

struct Traversal<'a, Reader, Visitor> {
    reader: &'a Reader,
    visited: HashSet<PageId>,
    overflow_pages: u64,
    verified_entries: u64,
    maximum_entries: u64,
    visitor: &'a mut Visitor,
}

impl<Reader: PageReader, Visitor: FnMut(&[u8], Vec<u8>) -> Result<()>>
    Traversal<'_, Reader, Visitor>
{
    fn verify_page(&mut self, page_id: PageId, depth: usize) -> Result<SubtreeSummary> {
        ensure_depth(depth)?;
        if self.visited.contains(&page_id) {
            return Err(Error::Corrupt(
                "a page is reachable through more than one reference".into(),
            ));
        }
        self.visited
            .try_reserve(1)
            .map_err(|_| Error::FileTooLarge)?;
        self.visited.insert(page_id);

        match self.reader.read_node(page_id)?.as_ref() {
            Node::Leaf(entries) => self.verify_leaf(entries),
            Node::PackedLeaf(leaf) => self.verify_leaf(&leaf.to_entries()?),
            Node::Branch {
                children,
                separators,
            } => self.verify_branch(children, separators, depth),
            Node::Overflow { .. } => Err(Error::Corrupt(
                "tree contains an overflow page as a node".into(),
            )),
        }
    }

    fn verify_branch(
        &mut self,
        children: &[PageId],
        separators: &[Vec<u8>],
        depth: usize,
    ) -> Result<SubtreeSummary> {
        let mut summaries = Vec::with_capacity(children.len());
        for child in children {
            summaries.push(self.verify_page(*child, depth + 1)?);
        }
        let first = summaries
            .first()
            .ok_or(Error::Corrupt("branch has no children".into()))?;
        let expected_depth = first.depth;
        if summaries
            .iter()
            .any(|summary| summary.depth != expected_depth)
        {
            return Err(Error::Corrupt("tree leaves have unequal depths".into()));
        }
        for (index, separator) in separators.iter().enumerate() {
            let left = summaries
                .get(index)
                .ok_or(Error::Corrupt("branch separator exceeds children".into()))?;
            let right = summaries
                .get(index + 1)
                .ok_or(Error::Corrupt("branch separator exceeds children".into()))?;
            if *separator != right.minimum {
                return Err(Error::Corrupt(
                    "branch separator does not match its child minimum".into(),
                ));
            }
            if left.maximum >= right.minimum {
                return Err(Error::Corrupt("child key ranges overlap".into()));
            }
        }
        let entries = summaries.iter().try_fold(0_u64, |total, summary| {
            total
                .checked_add(summary.entries)
                .ok_or(Error::FileTooLarge)
        })?;
        let last = summaries
            .last()
            .ok_or(Error::Corrupt("branch has no children".into()))?;
        Ok(SubtreeSummary {
            minimum: first.minimum.clone(),
            maximum: last.maximum.clone(),
            entries,
            depth: expected_depth
                .checked_add(1)
                .ok_or(Error::TreeDepthExceeded)?,
        })
    }

    fn verify_leaf(&mut self, entries: &[LeafEntry]) -> Result<SubtreeSummary> {
        for entry in entries {
            if self.verified_entries >= self.maximum_entries {
                return Err(Error::Corrupt(
                    "tree contains more entries than its metadata declares".into(),
                ));
            }
            let value = self.verify_value(&entry.value)?;
            (self.visitor)(&entry.key, value)?;
            self.verified_entries = self
                .verified_entries
                .checked_add(1)
                .ok_or(Error::FileTooLarge)?;
        }
        let first = entries
            .first()
            .ok_or(Error::Corrupt("leaf is empty".into()))?;
        let last = entries
            .last()
            .ok_or(Error::Corrupt("leaf is empty".into()))?;
        Ok(SubtreeSummary {
            minimum: first.key.to_vec(),
            maximum: last.key.to_vec(),
            entries: u64::try_from(entries.len()).map_err(|_| Error::FileTooLarge)?,
            depth: 1,
        })
    }

    fn verify_value(&mut self, value: &LeafValue) -> Result<Vec<u8>> {
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
        let maximum_pages = expected_len.div_ceil(OVERFLOW_DATA_BYTES);
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(expected_len)
            .map_err(|_| Error::FileTooLarge)?;
        let mut current = Some(*head);
        for _ in 0..maximum_pages {
            let page_id =
                current.ok_or_else(|| Error::Corrupt("overflow chain ended early".into()))?;
            if self.visited.contains(&page_id) {
                return Err(Error::Corrupt(
                    "an overflow page is reachable through more than one reference".into(),
                ));
            }
            self.visited
                .try_reserve(1)
                .map_err(|_| Error::FileTooLarge)?;
            self.visited.insert(page_id);
            self.overflow_pages = self
                .overflow_pages
                .checked_add(1)
                .ok_or(Error::FileTooLarge)?;
            current = match self.reader.read_node(page_id)?.as_ref() {
                Node::Overflow { next, bytes: chunk } => {
                    bytes.extend_from_slice(chunk);
                    *next
                }
                Node::Leaf(_) | Node::PackedLeaf(_) | Node::Branch { .. } => {
                    return Err(Error::Corrupt(
                        "overflow chain references a tree page".into(),
                    ));
                }
            };
        }
        if current.is_some() || bytes.len() != expected_len || crc32c(&bytes) != *checksum {
            return Err(Error::Corrupt("overflow value validation failed".into()));
        }
        Ok(bytes)
    }
}

fn ensure_depth(depth: usize) -> Result<()> {
    if depth >= MAX_TREE_DEPTH {
        Err(Error::TreeDepthExceeded)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::{verify, verify_with_entries};
    use crate::domain::{Head, PageId};
    use crate::infrastructure::page::{LeafEntry, LeafValue, Node};
    use crate::infrastructure::tree::PageReader;
    use crate::{Error, Result, TransactionId};

    #[test]
    fn repeated_page_reference_is_rejected() {
        let leaf = page_id(2);
        let root = page_id(3);
        let reader = MemoryReader::new([
            (
                leaf,
                Node::Leaf(vec![LeafEntry {
                    key: b"a".as_slice().into(),
                    value: LeafValue::Inline(b"one".as_slice().into()),
                }]),
            ),
            (
                root,
                Node::Branch {
                    children: vec![leaf, leaf],
                    separators: vec![b"b".to_vec()],
                },
            ),
        ]);
        let head = Head::new(TransactionId::from_raw(1), Some(root), 3, 2);

        assert!(
            matches!(verify(&reader, head), Err(Error::Corrupt(message)) if
            message == "a page is reachable through more than one reference")
        );
    }

    #[test]
    fn excess_entries_are_rejected_before_their_values_are_visited() {
        let root = page_id(2);
        let reader = MemoryReader::new([(
            root,
            Node::Leaf(vec![
                LeafEntry {
                    key: b"a".as_slice().into(),
                    value: LeafValue::Inline(b"one".as_slice().into()),
                },
                LeafEntry {
                    key: b"b".as_slice().into(),
                    value: LeafValue::Inline(b"two".as_slice().into()),
                },
            ]),
        )]);
        let head = Head::new(TransactionId::from_raw(1), Some(root), 2, 1);
        let mut visited = 0;

        assert!(matches!(verify_with_entries(&reader, head, |_, _| {
            visited += 1;
            Ok(())
        }), Err(Error::Corrupt(message)) if
            message == "tree contains more entries than its metadata declares"));
        assert_eq!(visited, 1);
    }

    fn page_id(raw: u64) -> PageId {
        PageId::from_raw(raw).unwrap()
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
