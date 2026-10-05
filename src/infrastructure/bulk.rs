use super::page::{LeafEntry, Node};
use super::tree::{PageWriter, store_value};
use crate::allocation::clone_bytes;
use crate::domain::{ChangeSet, PageId};
use crate::{Error, Result};

pub(crate) fn load(
    writer: &mut impl PageWriter,
    changes: &ChangeSet,
) -> Result<(Option<PageId>, u64)> {
    let mut loader = BulkLoader::new(writer);
    for (key, value) in changes.sorted_iter() {
        let value = value.ok_or(Error::InternalInvariant("bulk put value is missing"))?;
        loader.push(key, value)?;
    }
    loader.finish()
}

struct Child {
    page_id: PageId,
    minimum: Vec<u8>,
}

pub(crate) struct BulkLoader<'a, Writer> {
    writer: &'a mut Writer,
    leaf: Vec<LeafEntry>,
    leaves: Vec<Child>,
    entries: u64,
}

impl<'a, Writer: PageWriter> BulkLoader<'a, Writer> {
    pub(crate) fn new(writer: &'a mut Writer) -> Self {
        Self {
            writer,
            leaf: Vec::new(),
            leaves: Vec::new(),
            entries: 0,
        }
    }

    pub(crate) fn push(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        if self
            .leaf
            .last()
            .is_some_and(|last| last.key.as_ref() >= key)
        {
            return Err(Error::InternalInvariant(
                "bulk input keys are not strictly ordered",
            ));
        }
        let entries = self.entries.checked_add(1).ok_or(Error::FileTooLarge)?;
        self.leaf.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
        let key = clone_bytes(key)?.into_boxed_slice();
        let value = store_value(self.writer, value)?;
        let entry = LeafEntry { key, value };
        self.leaf.push(entry);
        if !Node::leaf_entries_fit(&self.leaf) {
            let overflow = self.leaf.pop().ok_or(Error::InternalInvariant(
                "bulk leaf overflow entry is missing",
            ))?;
            self.flush_leaf()?;
            self.leaf.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
            self.leaf.push(overflow);
        }
        self.entries = entries;
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<(Option<PageId>, u64)> {
        self.flush_leaf()?;
        let mut level = self.leaves;
        while level.len() > 1 {
            level = persist_branch_level(self.writer, level)?;
        }
        Ok((level.first().map(|child| child.page_id), self.entries))
    }

    fn flush_leaf(&mut self) -> Result<()> {
        if self.leaf.is_empty() {
            return Ok(());
        }
        let entries = std::mem::take(&mut self.leaf);
        let minimum = clone_bytes(&entries[0].key)?;
        let node = Node::Leaf(entries);
        self.leaves
            .try_reserve(1)
            .map_err(|_| Error::FileTooLarge)?;
        let page_id = self.writer.write_node(&node)?;
        self.leaf = match node {
            Node::Leaf(mut entries) => {
                entries.clear();
                entries
            }
            Node::PackedLeaf(_) | Node::Branch { .. } | Node::Overflow { .. } => {
                return Err(Error::InternalInvariant("bulk leaf changed node kind"));
            }
        };
        self.leaves.push(Child { page_id, minimum });
        Ok(())
    }
}

fn persist_branch_level(writer: &mut impl PageWriter, children: Vec<Child>) -> Result<Vec<Child>> {
    let mut output = Vec::new();
    let mut first = 0;
    while first < children.len() {
        let mut end = (first + 2).min(children.len());
        while end < children.len() && branch_node(&children[first..=end])?.fits() {
            end += 1;
        }
        if children.len() - end == 1 {
            end = end
                .checked_sub(1)
                .ok_or(Error::InternalInvariant("branch partition underflow"))?;
        }
        if end - first < 2 {
            return Err(Error::InternalInvariant("bulk branch fanout is too small"));
        }
        let group = &children[first..end];
        let node = branch_node(group)?;
        let minimum = clone_bytes(&group[0].minimum)?;
        output.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
        let page_id = writer.write_node(&node)?;
        output.push(Child { page_id, minimum });
        first = end;
    }
    Ok(output)
}

fn branch_node(children: &[Child]) -> Result<Node> {
    let separator_count = children.len().saturating_sub(1);
    let mut page_ids = Vec::new();
    page_ids
        .try_reserve_exact(children.len())
        .map_err(|_| Error::FileTooLarge)?;
    let mut separators = Vec::new();
    separators
        .try_reserve_exact(separator_count)
        .map_err(|_| Error::FileTooLarge)?;
    for (index, child) in children.iter().enumerate() {
        page_ids.push(child.page_id);
        if index != 0 {
            separators.push(clone_bytes(&child.minimum)?);
        }
    }
    Ok(Node::Branch {
        children: page_ids,
        separators,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::BulkLoader;
    use crate::domain::PageId;
    use crate::infrastructure::page::Node;
    use crate::infrastructure::tree::{self, PageReader, PageWriter};
    use crate::{Error, Result};

    #[test]
    fn borrowed_inputs_are_owned_and_remain_strictly_ordered() {
        let mut writer = MemoryWriter::new();
        let mut loader = BulkLoader::new(&mut writer);
        let mut key = b"a".to_vec();
        let mut value = b"first".to_vec();

        loader.push(&key, &value).unwrap();
        key[0] = b'b';
        value.copy_from_slice(b"other");
        loader.push(&key, &value).unwrap();
        assert!(matches!(
            loader.push(b"b", b"duplicate"),
            Err(Error::InternalInvariant(
                "bulk input keys are not strictly ordered"
            ))
        ));

        let (root, entries) = loader.finish().unwrap();
        assert_eq!(entries, 2);
        assert_eq!(
            tree::get(&writer, root, b"a").unwrap(),
            Some(b"first".to_vec())
        );
        assert_eq!(
            tree::get(&writer, root, b"b").unwrap(),
            Some(b"other".to_vec())
        );
    }

    struct MemoryWriter {
        nodes: BTreeMap<PageId, Arc<Node>>,
        next_page: u64,
    }

    impl MemoryWriter {
        fn new() -> Self {
            Self {
                nodes: BTreeMap::new(),
                next_page: 2,
            }
        }
    }

    impl PageReader for MemoryWriter {
        fn read_node(&self, page_id: PageId) -> Result<Arc<Node>> {
            self.nodes
                .get(&page_id)
                .cloned()
                .ok_or(Error::InternalInvariant("test page is missing"))
        }
    }

    impl PageWriter for MemoryWriter {
        fn write_node(&mut self, node: &Node) -> Result<PageId> {
            let page_id = PageId::from_raw(self.next_page).ok_or(Error::FileTooLarge)?;
            self.next_page += 1;
            self.nodes.insert(page_id, Arc::new(node.clone()));
            Ok(page_id)
        }
    }
}
