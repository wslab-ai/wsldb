use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{LeafSplitBias, PageReader, PageWriter, choose_leaf_split, insert_root};
use crate::domain::PageId;
use crate::infrastructure::page::{LeafEntry, LeafValue, Node};
use crate::infrastructure::tree_delete::delete_root;
use crate::{Error, Result};

#[test]
fn monotonic_leaf_splits_pack_the_completed_side() {
    let entries = (0_u16..38)
        .map(|index| LeafEntry {
            key: index.to_be_bytes().into(),
            value: LeafValue::Inline(vec![index as u8; 100].into()),
        })
        .collect::<Vec<_>>();
    assert!(!Node::leaf_entries_fit(&entries));

    let balanced = choose_leaf_split(&entries, LeafSplitBias::Balanced).unwrap();
    let prepend = choose_leaf_split(&entries, LeafSplitBias::Prepend).unwrap();
    let append = choose_leaf_split(&entries, LeafSplitBias::Append).unwrap();

    assert!(prepend < balanced);
    assert!(balanced < append);
    for split in [prepend, balanced, append] {
        assert!(Node::leaf_entries_fit(&entries[..split]));
        assert!(Node::leaf_entries_fit(&entries[split..]));
    }
}

#[test]
fn insert_propagates_known_minimum_without_reading_the_left_subtree() {
    let left = PageId::from_raw(2).unwrap();
    let right = PageId::from_raw(3).unwrap();
    let root = PageId::from_raw(4).unwrap();
    let mut writer = CountingWriter::new([
        (left, leaf(b"a", b"left")),
        (right, leaf(b"m", b"right")),
        (
            root,
            Node::Branch {
                children: vec![left, right],
                separators: vec![b"m".to_vec()],
            },
        ),
    ]);

    let (_, minimum, inserted) =
        insert_root(&mut writer, Some(root), Some(b"a"), b"z", b"new").unwrap();

    assert!(inserted);
    assert_eq!(minimum, b"a");
    assert_eq!(writer.reads.get(), 2);
}

#[test]
fn delete_propagates_known_minimum_without_reading_the_left_subtree() {
    let left = PageId::from_raw(2).unwrap();
    let right = PageId::from_raw(3).unwrap();
    let root = PageId::from_raw(4).unwrap();
    let left_minimum = vec![0, 0];
    let right_minimum = vec![1, 0];
    let mut writer = CountingWriter::new([
        (left, dense_leaf(0)),
        (right, dense_leaf(1)),
        (
            root,
            Node::Branch {
                children: vec![left, right],
                separators: vec![right_minimum],
            },
        ),
    ]);

    let (_, minimum, deleted) =
        delete_root(&mut writer, Some(root), Some(&left_minimum), &[1, 23]).unwrap();

    assert!(deleted);
    assert_eq!(minimum, Some(left_minimum));
    assert_eq!(writer.reads.get(), 3);
}

fn leaf(key: &[u8], value: &[u8]) -> Node {
    Node::Leaf(vec![LeafEntry {
        key: key.into(),
        value: LeafValue::Inline(value.into()),
    }])
}

fn dense_leaf(prefix: u8) -> Node {
    Node::Leaf(
        (0_u8..24)
            .map(|suffix| LeafEntry {
                key: vec![prefix, suffix].into(),
                value: LeafValue::Inline(vec![suffix; 100].into()),
            })
            .collect(),
    )
}

struct CountingWriter {
    nodes: BTreeMap<PageId, Arc<Node>>,
    next_page: u64,
    reads: Cell<u64>,
}

impl CountingWriter {
    fn new(nodes: impl IntoIterator<Item = (PageId, Node)>) -> Self {
        Self {
            nodes: nodes
                .into_iter()
                .map(|(page_id, node)| (page_id, Arc::new(node)))
                .collect(),
            next_page: 5,
            reads: Cell::new(0),
        }
    }
}

impl PageReader for CountingWriter {
    fn read_node(&self, page_id: PageId) -> Result<Arc<Node>> {
        self.reads.set(self.reads.get() + 1);
        self.nodes
            .get(&page_id)
            .cloned()
            .ok_or(Error::InternalInvariant("test page is missing"))
    }
}

impl PageWriter for CountingWriter {
    fn write_node(&mut self, node: &Node) -> Result<PageId> {
        let page_id = PageId::from_raw(self.next_page).ok_or(Error::FileTooLarge)?;
        self.next_page += 1;
        self.nodes.insert(page_id, Arc::new(node.clone()));
        Ok(page_id)
    }
}
