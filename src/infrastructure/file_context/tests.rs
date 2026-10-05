use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{CommitContext, read_node_from_file};
use crate::TransactionId;
use crate::domain::PageId;
use crate::infrastructure::metadata::PAGE_SIZE;
use crate::infrastructure::page::{LeafEntry, LeafValue, Node};
use crate::infrastructure::page_cache::PageCache;
use crate::infrastructure::tree::PageWriter;

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct TestFile {
    file: File,
    path: PathBuf,
}

impl TestFile {
    fn new() -> Self {
        let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "wsldb-{}-{sequence}-dense-page-remap",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len((2 * PAGE_SIZE) as u64).unwrap();
        Self { file, path }
    }
}

impl Drop for TestFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn reachable_pages_are_written_densely_and_references_are_remapped() {
    let file = TestFile::new();
    let cache = PageCache::new(0);
    let transaction = TransactionId::from_raw(1);
    let mut writer = CommitContext::new(&file.file, &cache, transaction, 2);

    let unreachable = writer.write_node(&leaf(b"unreachable")).unwrap();
    let left = writer.write_node(&leaf(b"left")).unwrap();
    let right = writer.write_node(&leaf(b"right")).unwrap();
    let virtual_root = writer
        .write_node(&Node::Branch {
            children: vec![left, right],
            separators: vec![b"right".to_vec()],
        })
        .unwrap();
    assert_eq!(unreachable.value(), 2);
    assert_eq!(virtual_root.value(), 5);

    let physical_root = writer.flush_reachable(Some(virtual_root)).unwrap().unwrap();

    assert_eq!(physical_root.value(), 4);
    assert_eq!(writer.next_physical_page(), 5);
    assert_eq!(file.file.metadata().unwrap().len(), (5 * PAGE_SIZE) as u64);
    let root = read_node_from_file(&file.file, physical_root, transaction, 4).unwrap();
    let Node::Branch {
        children,
        separators,
    } = root.as_ref()
    else {
        panic!("expected remapped branch root");
    };
    assert_eq!(
        children,
        &[PageId::from_raw(3).unwrap(), PageId::from_raw(2).unwrap()]
    );
    assert_eq!(separators, &[b"right".to_vec()]);

    assert_leaf_key(
        read_node_from_file(&file.file, children[0], transaction, 4)
            .unwrap()
            .as_ref(),
        b"left",
    );
    assert_leaf_key(
        read_node_from_file(&file.file, children[1], transaction, 4)
            .unwrap()
            .as_ref(),
        b"right",
    );
}

fn leaf(key: &[u8]) -> Node {
    Node::Leaf(vec![LeafEntry {
        key: key.into(),
        value: LeafValue::Inline(b"value".as_slice().into()),
    }])
}

fn assert_leaf_key(node: &Node, expected_key: &[u8]) {
    let entries = node.leaf_entries_owned().unwrap().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].key.as_ref(), expected_key);
    assert_eq!(
        entries[0].value,
        LeafValue::Inline(b"value".as_slice().into())
    );
}
