use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use super::bulk::BulkLoader;
use super::fault_injection;
use super::metadata::{self, METADATA_REGION_SIZE, PAGE_SIZE};
use super::page::Node;
use super::positioned_io::{read_exact_at, write_all_at};
use super::snapshot_file;
use super::tree::{self, PageReader, PageWriter};
use super::tree_verify::{self, LogicalSnapshot};
use crate::allocation::clone_bytes;
use crate::domain::{ChangeSet, Head, PageId};
use crate::{CompactionReport, Error, Result, TransactionId, VerificationReport};

const MAINTENANCE_CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAINTENANCE_CHUNK_ENTRIES: usize = 32 * 1024;

pub(crate) fn compact(
    source: &impl PageReader,
    source_head: Head,
    compacted_transaction_id: TransactionId,
    source_bytes: u64,
    target: &Path,
) -> Result<CompactionReport> {
    let (destination, temporary) = snapshot_file::create_temporary(target)?;
    destination.set_len(METADATA_REGION_SIZE)?;
    let mut writer = DirectWriter::new(&destination, compacted_transaction_id);
    let mut loader = BulkLoader::new(&mut writer);
    let mut source_logical = LogicalSnapshot::new();
    tree_verify::verify_with_entries(source, source_head, |key, value| {
        source_logical.push(key, &value)?;
        loader.push(key, &value)
    })?;

    let (root, entries) = loader.finish()?;
    let (source_entries, source_hash) = source_logical.finish_summary();
    if entries != source_head.entry_count() {
        return Err(Error::Corrupt(
            "compaction input count does not match source metadata".into(),
        ));
    }
    if source_entries != entries {
        return Err(Error::InternalInvariant(
            "compaction logical count differs from bulk loader",
        ));
    }
    let last_page = writer.last_page();
    let compacted_head = Head::new(compacted_transaction_id, root, last_page, entries);
    fault_injection::before("compaction_data_sync")?;
    destination.sync_data()?;
    super::crash_injection::hit("compaction_data_sync");
    fault_injection::before("compaction_metadata_write")?;
    write_all_at(
        &destination,
        &metadata::encode(compacted_head),
        metadata::slot_offset(compacted_head.transaction_id()),
    )?;
    super::crash_injection::hit("compaction_metadata_write");
    fault_injection::before("compaction_metadata_sync")?;
    destination.sync_all()?;
    super::crash_injection::hit("compaction_metadata_sync");
    let compacted_snapshot = tree::verify(&writer, compacted_head)?;
    ensure_logical_equivalence(source_entries, source_hash, &compacted_snapshot)?;
    let compacted_bytes = last_page
        .checked_add(1)
        .and_then(|pages| pages.checked_mul(PAGE_SIZE as u64))
        .ok_or(Error::FileTooLarge)?;
    fault_injection::before("compaction_truncate")?;
    destination.set_len(compacted_bytes)?;
    super::crash_injection::hit("compaction_truncate");
    fault_injection::before("compaction_final_sync")?;
    destination.sync_all()?;
    super::crash_injection::hit("compaction_final_sync");
    snapshot_file::publish(destination, temporary, target)?;

    Ok(CompactionReport {
        source_transaction_id: source_head.transaction_id(),
        compacted_snapshot,
        source_bytes,
        compacted_bytes,
        publication_pause: Duration::ZERO,
    })
}

pub(crate) fn compact_layered(
    source: &impl PageReader,
    physical_head: Head,
    logical_head: Head,
    changes: ChangeSet,
    compacted_transaction_id: TransactionId,
    source_bytes: u64,
    target: &Path,
) -> Result<CompactionReport> {
    let (destination, temporary) = snapshot_file::create_temporary(target)?;
    destination.set_len(METADATA_REGION_SIZE)?;
    let mut writer = DirectWriter::new(&destination, compacted_transaction_id);
    let mut loader = BulkLoader::new(&mut writer);
    let mut chunk = MaintenanceChunk::new();
    let mut changes = changes.into_sorted_iter().peekable();
    tree_verify::verify_with_entries(source, physical_head, |key, value| {
        while changes
            .peek()
            .is_some_and(|(changed, _)| changed.as_ref() < key)
        {
            let (changed, value) = changes.next().ok_or(Error::InternalInvariant(
                "compaction overlay iterator changed",
            ))?;
            if let Some(value) = value {
                chunk.push_shared(&mut loader, changed, value)?;
            }
        }
        if changes
            .peek()
            .is_some_and(|(changed, _)| changed.as_ref() == key)
        {
            let (changed, value) = changes.next().ok_or(Error::InternalInvariant(
                "compaction overlay iterator changed",
            ))?;
            if let Some(value) = value {
                chunk.push_shared(&mut loader, changed, value)?;
            }
        } else {
            chunk.push_borrowed(&mut loader, key, value)?;
        }
        Ok(())
    })?;
    for (key, value) in changes {
        if let Some(value) = value {
            chunk.push_shared(&mut loader, key, value)?;
        }
    }
    chunk.flush(&mut loader)?;
    let (source_entries, source_hash) = chunk.finish_summary();
    let (root, entry_count) = loader.finish()?;
    if entry_count != logical_head.entry_count() {
        return Err(Error::Corrupt(
            "compaction input count does not match source metadata".into(),
        ));
    }
    if source_entries != entry_count {
        return Err(Error::InternalInvariant(
            "compaction logical count differs from bulk loader",
        ));
    }
    let last_page = writer.last_page();
    let compacted_head = Head::new(compacted_transaction_id, root, last_page, entry_count);
    fault_injection::before("compaction_data_sync")?;
    destination.sync_data()?;
    super::crash_injection::hit("compaction_data_sync");
    fault_injection::before("compaction_metadata_write")?;
    write_all_at(
        &destination,
        &metadata::encode(compacted_head),
        metadata::slot_offset(compacted_head.transaction_id()),
    )?;
    super::crash_injection::hit("compaction_metadata_write");
    fault_injection::before("compaction_metadata_sync")?;
    destination.sync_all()?;
    super::crash_injection::hit("compaction_metadata_sync");
    let compacted_snapshot = tree::verify(&writer, compacted_head)?;
    ensure_logical_equivalence(source_entries, source_hash, &compacted_snapshot)?;
    let compacted_bytes = last_page
        .checked_add(1)
        .and_then(|pages| pages.checked_mul(PAGE_SIZE as u64))
        .ok_or(Error::FileTooLarge)?;
    fault_injection::before("compaction_truncate")?;
    destination.set_len(compacted_bytes)?;
    super::crash_injection::hit("compaction_truncate");
    fault_injection::before("compaction_final_sync")?;
    destination.sync_all()?;
    super::crash_injection::hit("compaction_final_sync");
    snapshot_file::publish(destination, temporary, target)?;

    Ok(CompactionReport {
        source_transaction_id: logical_head.transaction_id(),
        compacted_snapshot,
        source_bytes,
        compacted_bytes,
        publication_pause: Duration::ZERO,
    })
}

struct MaintenanceChunk {
    entries: Vec<(MaintenanceKey, Vec<u8>)>,
    payload_bytes: usize,
    logical: LogicalSnapshot,
}

enum MaintenanceKey {
    Owned(Vec<u8>),
    Shared(Arc<[u8]>),
}

impl AsRef<[u8]> for MaintenanceKey {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Owned(key) => key,
            Self::Shared(key) => key,
        }
    }
}

impl MaintenanceChunk {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            payload_bytes: 0,
            logical: LogicalSnapshot::new(),
        }
    }

    fn push_borrowed(
        &mut self,
        loader: &mut BulkLoader<'_, impl PageWriter>,
        key: &[u8],
        value: Vec<u8>,
    ) -> Result<()> {
        self.push(loader, MaintenanceKey::Owned(clone_bytes(key)?), value)
    }

    fn push_shared(
        &mut self,
        loader: &mut BulkLoader<'_, impl PageWriter>,
        key: Arc<[u8]>,
        value: Vec<u8>,
    ) -> Result<()> {
        self.push(loader, MaintenanceKey::Shared(key), value)
    }

    fn push(
        &mut self,
        loader: &mut BulkLoader<'_, impl PageWriter>,
        key: MaintenanceKey,
        value: Vec<u8>,
    ) -> Result<()> {
        let bytes = key
            .as_ref()
            .len()
            .checked_add(value.len())
            .ok_or(Error::FileTooLarge)?;
        if !self.entries.is_empty()
            && (self.entries.len() == MAINTENANCE_CHUNK_ENTRIES
                || self.payload_bytes.saturating_add(bytes) > MAINTENANCE_CHUNK_BYTES)
        {
            self.flush(loader)?;
        }
        self.entries
            .try_reserve(1)
            .map_err(|_| Error::FileTooLarge)?;
        self.payload_bytes = self
            .payload_bytes
            .checked_add(bytes)
            .ok_or(Error::FileTooLarge)?;
        self.logical.push(key.as_ref(), &value)?;
        self.entries.push((key, value));
        Ok(())
    }

    fn flush(&mut self, loader: &mut BulkLoader<'_, impl PageWriter>) -> Result<()> {
        for (key, value) in self.entries.drain(..) {
            loader.push(key.as_ref(), &value)?;
        }
        self.payload_bytes = 0;
        Ok(())
    }

    fn finish_summary(self) -> (u64, [u8; 32]) {
        self.logical.finish_summary()
    }
}

fn ensure_logical_equivalence(
    source_entries: u64,
    source_hash: [u8; 32],
    compacted: &VerificationReport,
) -> Result<()> {
    if compacted.entries != source_entries || compacted.logical_hash != source_hash {
        return Err(Error::Corrupt(
            "compaction output does not match its source logical snapshot".into(),
        ));
    }
    Ok(())
}

struct DirectWriter<'a> {
    file: &'a File,
    transaction_id: TransactionId,
    next_page: u64,
}

impl<'a> DirectWriter<'a> {
    const fn new(file: &'a File, transaction_id: TransactionId) -> Self {
        Self {
            file,
            transaction_id,
            next_page: PageId::FIRST_DATA_PAGE,
        }
    }

    const fn last_page(&self) -> u64 {
        self.next_page.saturating_sub(1)
    }
}

impl PageReader for DirectWriter<'_> {
    fn read_node(&self, page_id: PageId) -> Result<Arc<Node>> {
        if page_id.value() >= self.next_page {
            return Err(Error::InternalInvariant(
                "compaction page has not been written",
            ));
        }
        let mut page = [0_u8; PAGE_SIZE];
        read_exact_at(self.file, &mut page, page_offset(page_id)?)?;
        Node::decode(&page, page_id, self.transaction_id).map(Arc::new)
    }
}

impl PageWriter for DirectWriter<'_> {
    fn write_node(&mut self, node: &Node) -> Result<PageId> {
        let page_id = PageId::from_raw(self.next_page).ok_or(Error::FileTooLarge)?;
        let next_page = self.next_page.checked_add(1).ok_or(Error::FileTooLarge)?;
        write_all_at(
            self.file,
            &node.encode(page_id, self.transaction_id)?,
            page_offset(page_id)?,
        )?;
        super::crash_injection::hit("compaction_page_write");
        self.next_page = next_page;
        Ok(page_id)
    }
}

fn page_offset(page_id: PageId) -> Result<u64> {
    page_id
        .value()
        .checked_mul(PAGE_SIZE as u64)
        .ok_or(Error::FileTooLarge)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{compact, compact_layered, ensure_logical_equivalence};
    use crate::domain::{ChangeSet, Head, PageId};
    use crate::infrastructure::page::{LeafEntry, LeafValue, Node};
    use crate::infrastructure::tree::PageReader;
    use crate::{Error, Result, TransactionId, VerificationReport};

    static NEXT_TEST_TARGET: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn compaction_rejects_a_logically_different_output() {
        let report = VerificationReport {
            transaction_id: TransactionId::from_raw(7),
            entries: 2,
            tree_pages: 1,
            overflow_pages: 0,
            tree_depth: 1,
            logical_hash: [9; 32],
        };

        assert!(ensure_logical_equivalence(2, [9; 32], &report).is_ok());
        assert!(matches!(
            ensure_logical_equivalence(1, [9; 32], &report),
            Err(Error::Corrupt(message))
                if message == "compaction output does not match its source logical snapshot"
        ));
        assert!(matches!(
            ensure_logical_equivalence(2, [8; 32], &report),
            Err(Error::Corrupt(message))
                if message == "compaction output does not match its source logical snapshot"
        ));
    }

    #[test]
    fn compaction_rejects_invalid_source_structure_before_publication() {
        let root = page_id(4);
        let reader = MemoryReader::new([
            (
                root,
                Node::Branch {
                    children: vec![page_id(2), page_id(3)],
                    separators: vec![b"b".to_vec()],
                },
            ),
            (page_id(2), leaf(b"a", b"one")),
            (page_id(3), leaf(b"c", b"three")),
        ]);
        let source_head = Head::new(TransactionId::from_raw(1), Some(root), 4, 2);
        let direct_target = TestTarget::new();

        assert!(matches!(
            compact(
                &reader,
                source_head,
                TransactionId::from_raw(2),
                0,
                &direct_target.0,
            ),
            Err(Error::Corrupt(message))
                if message == "branch separator does not match its child minimum"
        ));
        assert!(!direct_target.0.exists());

        let layered_target = TestTarget::new();
        assert!(matches!(
            compact_layered(
                &reader,
                source_head,
                source_head,
                ChangeSet::new(),
                TransactionId::from_raw(2),
                0,
                &layered_target.0,
            ),
            Err(Error::Corrupt(message))
                if message == "branch separator does not match its child minimum"
        ));
        assert!(!layered_target.0.exists());
    }

    fn page_id(raw: u64) -> PageId {
        PageId::from_raw(raw).expect("test page ID must be valid")
    }

    fn leaf(key: &[u8], value: &[u8]) -> Node {
        Node::Leaf(vec![LeafEntry {
            key: key.into(),
            value: LeafValue::Inline(value.into()),
        }])
    }

    struct MemoryReader(BTreeMap<PageId, Arc<Node>>);

    impl MemoryReader {
        fn new(nodes: impl IntoIterator<Item = (PageId, Node)>) -> Self {
            Self(
                nodes
                    .into_iter()
                    .map(|(page_id, node)| (page_id, Arc::new(node)))
                    .collect(),
            )
        }
    }

    impl PageReader for MemoryReader {
        fn read_node(&self, page_id: PageId) -> Result<Arc<Node>> {
            self.0
                .get(&page_id)
                .cloned()
                .ok_or(Error::InternalInvariant("test page is missing"))
        }
    }

    struct TestTarget(PathBuf);

    impl TestTarget {
        fn new() -> Self {
            let sequence = NEXT_TEST_TARGET.fetch_add(1, Ordering::Relaxed);
            Self(std::env::temp_dir().join(format!(
                "wsldb-invalid-compaction-{}-{sequence}",
                std::process::id()
            )))
        }
    }

    impl Drop for TestTarget {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}
