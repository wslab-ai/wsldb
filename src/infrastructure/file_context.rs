use std::fs::File;
use std::sync::Arc;

use super::crash_injection;
use super::fault_injection;
use super::metadata::{self, METADATA_REGION_SIZE, METADATA_SLOTS, PAGE_SIZE};
use super::page::{LeafValue, Node};
use super::page_cache::PageCache;
use super::positioned_io::{read_exact_at, write_all_at};
use super::tree::{PageReader, PageWriter};
use crate::domain::{Head, PageId};
use crate::{Error, Result, TransactionId};

pub(crate) struct ReadContext<'a> {
    file: &'a File,
    page_cache: &'a PageCache,
    head: Head,
}

impl<'a> ReadContext<'a> {
    pub(crate) const fn new(file: &'a File, page_cache: &'a PageCache, head: Head) -> Self {
        Self {
            file,
            page_cache,
            head,
        }
    }
}

impl PageReader for ReadContext<'_> {
    fn read_node(&self, page_id: PageId) -> Result<Arc<Node>> {
        read_node_cached(
            self.file,
            self.page_cache,
            page_id,
            self.head.transaction_id(),
            self.head.last_page(),
        )
    }

    fn inspect_node<T>(
        &self,
        page_id: PageId,
        inspect: impl FnOnce(&Node) -> Result<T>,
    ) -> Result<T> {
        if page_id.value() > self.head.last_page() {
            return Err(Error::Corrupt(
                "page reference exceeds the committed boundary".into(),
            ));
        }
        self.page_cache.inspect(
            page_id,
            || {
                read_node_from_file(
                    self.file,
                    page_id,
                    self.head.transaction_id(),
                    self.head.last_page(),
                )
            },
            inspect,
        )
    }
}

pub(crate) struct UncachedReadContext<'a> {
    file: &'a File,
    head: Head,
}

impl<'a> UncachedReadContext<'a> {
    pub(crate) const fn new(file: &'a File, head: Head) -> Self {
        Self { file, head }
    }
}

impl PageReader for UncachedReadContext<'_> {
    fn read_node(&self, page_id: PageId) -> Result<Arc<Node>> {
        read_node_from_file(
            self.file,
            page_id,
            self.head.transaction_id(),
            self.head.last_page(),
        )
    }
}

pub(crate) struct CommitContext<'a> {
    file: &'a File,
    page_cache: &'a PageCache,
    transaction_id: TransactionId,
    first_new_page: u64,
    next_virtual_page: u64,
    next_physical_page: u64,
    pending: Vec<Arc<Node>>,
    cache_on_flush: Vec<bool>,
    leaf_page_bytes: u64,
    branch_page_bytes: u64,
    overflow_page_bytes: u64,
}

/// Writes an already ordered, immutable tree directly to its final page IDs.
///
/// Unlike [`CommitContext`], this context does not retain encoded tree pages
/// for a later reachability pass. It is only valid for builders whose output
/// references pages that have already been written, such as a bottom-up bulk
/// load into an empty tree.
pub(crate) struct StreamingCommitContext<'a> {
    file: &'a File,
    page_cache: &'a PageCache,
    transaction_id: TransactionId,
    next_page: u64,
    buffered_start_page: u64,
    buffered_pages: Vec<u8>,
    buffered_branches: Vec<(PageId, Arc<Node>)>,
    leaf_page_bytes: u64,
    branch_page_bytes: u64,
    overflow_page_bytes: u64,
}

const STREAM_WRITE_BUFFER_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct PageWriteBytes {
    pub(crate) leaf: u64,
    pub(crate) branch: u64,
    pub(crate) overflow: u64,
}

impl PageWriteBytes {
    pub(crate) fn total(self) -> Result<u64> {
        self.leaf
            .checked_add(self.branch)
            .and_then(|bytes| bytes.checked_add(self.overflow))
            .ok_or(Error::FileTooLarge)
    }
}

impl<'a> StreamingCommitContext<'a> {
    pub(crate) const fn new(
        file: &'a File,
        page_cache: &'a PageCache,
        transaction_id: TransactionId,
        next_page: u64,
    ) -> Self {
        Self {
            file,
            page_cache,
            transaction_id,
            next_page,
            buffered_start_page: next_page,
            buffered_pages: Vec::new(),
            buffered_branches: Vec::new(),
            leaf_page_bytes: 0,
            branch_page_bytes: 0,
            overflow_page_bytes: 0,
        }
    }

    pub(crate) const fn next_physical_page(&self) -> u64 {
        self.next_page
    }

    pub(crate) const fn page_write_bytes(&self) -> PageWriteBytes {
        PageWriteBytes {
            leaf: self.leaf_page_bytes,
            branch: self.branch_page_bytes,
            overflow: self.overflow_page_bytes,
        }
    }

    pub(crate) fn flush_pages(&mut self) -> Result<()> {
        if self.buffered_pages.is_empty() {
            return Ok(());
        }
        let page_id = PageId::from_raw(self.buffered_start_page).ok_or(Error::FileTooLarge)?;
        fault_injection::before("checkpoint_stream_write")?;
        write_all_at(self.file, &self.buffered_pages, page_offset(page_id)?)?;
        crash_injection::hit("checkpoint_stream_write");
        for (page_id, node) in &self.buffered_branches {
            self.page_cache.insert(*page_id, Arc::clone(node))?;
        }
        self.buffered_pages.clear();
        self.buffered_branches.clear();
        self.buffered_start_page = self.next_page;
        Ok(())
    }

    fn record_page_write(&mut self, node: &Node) -> Result<()> {
        let counter = match node {
            Node::Leaf(_) | Node::PackedLeaf(_) => &mut self.leaf_page_bytes,
            Node::Branch { .. } => &mut self.branch_page_bytes,
            Node::Overflow { .. } => &mut self.overflow_page_bytes,
        };
        *counter = counter
            .checked_add(PAGE_SIZE as u64)
            .ok_or(Error::FileTooLarge)?;
        Ok(())
    }
}

impl PageReader for StreamingCommitContext<'_> {
    fn read_node(&self, page_id: PageId) -> Result<Arc<Node>> {
        if page_id.value() >= self.buffered_start_page && page_id.value() < self.next_page {
            return Err(Error::InternalInvariant(
                "streaming writer cannot read an unflushed page",
            ));
        }
        let maximum_page = self
            .next_page
            .checked_sub(1)
            .ok_or(Error::InternalInvariant("streaming page boundary"))?;
        read_node_from_file(self.file, page_id, self.transaction_id, maximum_page)
    }
}

impl PageWriter for StreamingCommitContext<'_> {
    fn write_node(&mut self, node: &Node) -> Result<PageId> {
        let page_id = PageId::from_raw(self.next_page).ok_or(Error::FileTooLarge)?;
        let page = node.encode(page_id, self.transaction_id)?;
        if matches!(node, Node::Branch { .. }) {
            self.buffered_branches
                .try_reserve(1)
                .map_err(|_| Error::FileTooLarge)?;
        }
        self.buffered_pages
            .try_reserve(page.len())
            .map_err(|_| Error::FileTooLarge)?;
        self.buffered_pages.extend_from_slice(&page);
        self.record_page_write(node)?;
        if matches!(node, Node::Branch { .. }) {
            self.buffered_branches
                .push((page_id, Arc::new(node.clone())));
        }
        self.next_page = self.next_page.checked_add(1).ok_or(Error::FileTooLarge)?;
        if self.buffered_pages.len() >= STREAM_WRITE_BUFFER_BYTES {
            self.flush_pages()?;
        }
        #[cfg(feature = "crash-injection")]
        self.flush_pages()?;
        crash_injection::hit("page_write");
        Ok(page_id)
    }
}

impl<'a> CommitContext<'a> {
    pub(crate) const fn new(
        file: &'a File,
        page_cache: &'a PageCache,
        transaction_id: TransactionId,
        next_page: u64,
    ) -> Self {
        Self {
            file,
            page_cache,
            transaction_id,
            first_new_page: next_page,
            next_virtual_page: next_page,
            next_physical_page: next_page,
            pending: Vec::new(),
            cache_on_flush: Vec::new(),
            leaf_page_bytes: 0,
            branch_page_bytes: 0,
            overflow_page_bytes: 0,
        }
    }

    pub(crate) const fn next_physical_page(&self) -> u64 {
        self.next_physical_page
    }

    pub(crate) const fn page_write_bytes(&self) -> PageWriteBytes {
        PageWriteBytes {
            leaf: self.leaf_page_bytes,
            branch: self.branch_page_bytes,
            overflow: self.overflow_page_bytes,
        }
    }

    pub(crate) fn flush_reachable(&mut self, root: Option<PageId>) -> Result<Option<PageId>> {
        let reachable = self.reachable_postorder(root)?;
        let mut remapped = Vec::new();
        remapped
            .try_reserve_exact(self.pending.len())
            .map_err(|_| Error::FileTooLarge)?;
        remapped.resize(self.pending.len(), None);

        for virtual_page in reachable {
            let virtual_index =
                virtual_page_index(virtual_page, self.first_new_page, self.pending.len())?;
            let cache_on_flush = *self
                .cache_on_flush
                .get(virtual_index)
                .ok_or(Error::InternalInvariant("page cache marker is missing"))?;
            if remapped.get(virtual_index).is_none_or(Option::is_some) {
                return Err(Error::InternalInvariant("page remap slot is invalid"));
            }
            let node = self
                .pending
                .get(virtual_index)
                .ok_or(Error::InternalInvariant("pending page is missing"))?;
            let physical_page =
                PageId::from_raw(self.next_physical_page).ok_or(Error::FileTooLarge)?;
            let next_physical_page = self
                .next_physical_page
                .checked_add(1)
                .ok_or(Error::FileTooLarge)?;
            let node = Arc::new(remap_node(node, self.first_new_page, &remapped)?);
            let page = node.encode(physical_page, self.transaction_id)?;
            self.record_page_write(node.as_ref())?;
            write_all_at(self.file, &page, page_offset(physical_page)?)?;
            crash_injection::hit("page_write");
            if matches!(node.as_ref(), Node::Branch { .. }) || cache_on_flush {
                self.page_cache.insert(physical_page, Arc::clone(&node))?;
            }
            let slot = remapped
                .get_mut(virtual_index)
                .ok_or(Error::InternalInvariant("page remap slot is missing"))?;
            if slot.replace(physical_page).is_some() {
                return Err(Error::InternalInvariant("page was remapped twice"));
            }
            self.next_physical_page = next_physical_page;
        }

        root.map(|page_id| remap_page_id(page_id, self.first_new_page, &remapped))
            .transpose()
    }

    fn record_page_write(&mut self, node: &Node) -> Result<()> {
        let counter = match node {
            Node::Leaf(_) | Node::PackedLeaf(_) => &mut self.leaf_page_bytes,
            Node::Branch { .. } => &mut self.branch_page_bytes,
            Node::Overflow { .. } => &mut self.overflow_page_bytes,
        };
        *counter = counter
            .checked_add(PAGE_SIZE as u64)
            .ok_or(Error::FileTooLarge)?;
        Ok(())
    }

    fn mark_cache_on_flush(&mut self, page_id: PageId) -> Result<()> {
        let index = virtual_page_index(page_id, self.first_new_page, self.pending.len())?;
        let cache = self
            .cache_on_flush
            .get_mut(index)
            .ok_or(Error::InternalInvariant("page cache marker is missing"))?;
        *cache = true;
        Ok(())
    }

    fn reachable_postorder(&self, root: Option<PageId>) -> Result<Vec<PageId>> {
        let mut visited = Vec::new();
        visited
            .try_reserve_exact(self.pending.len())
            .map_err(|_| Error::FileTooLarge)?;
        visited.resize(self.pending.len(), false);
        let mut postorder = Vec::new();
        postorder
            .try_reserve_exact(self.pending.len())
            .map_err(|_| Error::FileTooLarge)?;
        let mut pending = Vec::new();
        if let Some(root) = root {
            pending.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
            pending.push((root, false));
        }
        while let Some((page_id, expanded)) = pending.pop() {
            if page_id.value() < self.first_new_page {
                continue;
            }
            let page_index = virtual_page_index(page_id, self.first_new_page, self.pending.len())?;
            if expanded {
                postorder.push(page_id);
                continue;
            }
            let seen = visited
                .get_mut(page_index)
                .ok_or(Error::InternalInvariant("visited page slot is missing"))?;
            if *seen {
                continue;
            }
            *seen = true;
            let node = self
                .pending
                .get(page_index)
                .ok_or(Error::InternalInvariant("pending page is missing"))?;
            pending.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
            pending.push((page_id, true));
            push_page_references(node, &mut pending)?;
        }
        Ok(postorder)
    }
}

impl PageReader for CommitContext<'_> {
    fn read_node(&self, page_id: PageId) -> Result<Arc<Node>> {
        if page_id.value() >= self.first_new_page {
            let index = virtual_page_index(page_id, self.first_new_page, self.pending.len())?;
            return self
                .pending
                .get(index)
                .cloned()
                .ok_or(Error::InternalInvariant("pending page is missing"));
        }
        let maximum_page = self
            .first_new_page
            .checked_sub(1)
            .ok_or(Error::InternalInvariant("commit page boundary"))?;
        read_node_cached(
            self.file,
            self.page_cache,
            page_id,
            self.transaction_id,
            maximum_page,
        )
    }
}

impl PageWriter for CommitContext<'_> {
    fn write_node(&mut self, node: &Node) -> Result<PageId> {
        let page_id = PageId::from_raw(self.next_virtual_page).ok_or(Error::FileTooLarge)?;
        let next_virtual_page = self
            .next_virtual_page
            .checked_add(1)
            .ok_or(Error::FileTooLarge)?;
        self.pending
            .try_reserve(1)
            .map_err(|_| Error::FileTooLarge)?;
        self.cache_on_flush
            .try_reserve(1)
            .map_err(|_| Error::FileTooLarge)?;
        self.pending.push(Arc::new(node.clone()));
        self.cache_on_flush.push(false);
        self.next_virtual_page = next_virtual_page;
        Ok(page_id)
    }

    fn write_hot_node(&mut self, node: &Node) -> Result<PageId> {
        let page_id = self.write_node(node)?;
        self.mark_cache_on_flush(page_id)?;
        Ok(page_id)
    }

    fn replace_node(&mut self, page_id: PageId, node: &Node) -> Result<PageId> {
        if page_id.value() < self.first_new_page {
            let replacement = self.write_node(node)?;
            self.mark_cache_on_flush(replacement)?;
            return Ok(replacement);
        }
        let index = virtual_page_index(page_id, self.first_new_page, self.pending.len())?;
        let pending = self.pending.get_mut(index).ok_or(Error::InternalInvariant(
            "pending replacement page is missing",
        ))?;
        *pending = Arc::new(node.clone());
        self.mark_cache_on_flush(page_id)?;
        Ok(page_id)
    }
}

pub(crate) fn load_head(file: &File) -> Result<Head> {
    load_head_candidates(file)?
        .last()
        .copied()
        .ok_or_else(|| Error::Corrupt("no valid metadata slot remains".into()))
}

pub(crate) fn load_head_candidates(file: &File) -> Result<Vec<Head>> {
    let file_len = file.metadata()?.len();
    if file_len < METADATA_REGION_SIZE {
        return Err(Error::Corrupt("metadata region is truncated".into()));
    }

    let mut candidates = Vec::with_capacity(METADATA_SLOTS);
    for slot in 0..METADATA_SLOTS {
        if let Some(head) = read_head_candidate(file, file_len, slot)? {
            candidates.push(head);
        }
    }
    candidates.sort_by_key(|head| head.transaction_id());

    match candidates.as_slice() {
        [] => Err(Error::Corrupt("no valid metadata slot remains".into())),
        [older, newer] if older.transaction_id() == newer.transaction_id() && older != newer => {
            Err(Error::Corrupt(
                "metadata slots disagree for the same transaction".into(),
            ))
        }
        [_] | [_, _] => Ok(candidates),
        _ => Err(Error::InternalInvariant("metadata candidate count")),
    }
}

fn push_page_references(node: &Node, pending: &mut Vec<(PageId, bool)>) -> Result<()> {
    let mut push = |page_id| -> Result<()> {
        pending.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
        pending.push((page_id, false));
        Ok(())
    };
    match node {
        Node::Leaf(entries) => {
            for entry in entries {
                if let LeafValue::Overflow { head, .. } = &entry.value {
                    push(*head)?;
                }
            }
        }
        Node::PackedLeaf(leaf) => {
            for head in leaf.overflow_heads() {
                push(head)?;
            }
        }
        Node::Branch { children, .. } => {
            pending
                .try_reserve(children.len())
                .map_err(|_| Error::FileTooLarge)?;
            pending.extend(children.iter().copied().map(|page_id| (page_id, false)));
        }
        Node::Overflow { next, .. } => {
            if let Some(next) = next {
                push(*next)?;
            }
        }
    }
    Ok(())
}

fn remap_node(node: &Node, first_new_page: u64, remapped: &[Option<PageId>]) -> Result<Node> {
    let mut node = match node {
        Node::PackedLeaf(leaf) => Node::Leaf(leaf.to_entries()?),
        _ => node.clone(),
    };
    match &mut node {
        Node::Leaf(entries) => {
            for entry in entries {
                if let LeafValue::Overflow { head, .. } = &mut entry.value {
                    *head = remap_page_id(*head, first_new_page, remapped)?;
                }
            }
        }
        Node::Branch { children, .. } => {
            for child in children {
                *child = remap_page_id(*child, first_new_page, remapped)?;
            }
        }
        Node::Overflow { next, .. } => {
            if let Some(page_id) = next {
                *page_id = remap_page_id(*page_id, first_new_page, remapped)?;
            }
        }
        Node::PackedLeaf(_) => {
            return Err(Error::InternalInvariant("packed leaf was not materialized"));
        }
    }
    Ok(node)
}

fn remap_page_id(
    page_id: PageId,
    first_new_page: u64,
    remapped: &[Option<PageId>],
) -> Result<PageId> {
    if page_id.value() < first_new_page {
        Ok(page_id)
    } else {
        let index = virtual_page_index(page_id, first_new_page, remapped.len())?;
        remapped
            .get(index)
            .copied()
            .flatten()
            .ok_or(Error::InternalInvariant("reachable page was not remapped"))
    }
}

fn virtual_page_index(page_id: PageId, first_new_page: u64, page_count: usize) -> Result<usize> {
    let index = page_id
        .value()
        .checked_sub(first_new_page)
        .and_then(|index| usize::try_from(index).ok())
        .filter(|index| *index < page_count)
        .ok_or(Error::InternalInvariant("pending page is missing"))?;
    Ok(index)
}

fn read_head_candidate(file: &File, file_len: u64, slot: usize) -> Result<Option<Head>> {
    let mut page = [0_u8; PAGE_SIZE];
    let offset = u64::try_from(slot)
        .ok()
        .and_then(|slot| slot.checked_mul(PAGE_SIZE as u64))
        .ok_or(Error::FileTooLarge)?;
    read_exact_at(file, &mut page, offset)?;
    let Some(head) = metadata::decode(&page)? else {
        return Ok(None);
    };
    if offset != metadata::slot_offset(head.transaction_id()) {
        return Ok(None);
    }

    let file_pages = file_len / PAGE_SIZE as u64;
    if head.last_page() >= file_pages {
        return Ok(None);
    }
    if let Some(root) = head.root() {
        match read_node_from_file(file, root, head.transaction_id(), head.last_page()) {
            Ok(node) => match node.as_ref() {
                Node::Leaf(_) | Node::PackedLeaf(_) | Node::Branch { .. } => {}
                Node::Overflow { .. } => return Ok(None),
            },
            Err(Error::Corrupt(_)) => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(Some(head))
}

fn read_node_cached(
    file: &File,
    page_cache: &PageCache,
    page_id: PageId,
    maximum_transaction_id: TransactionId,
    maximum_page: u64,
) -> Result<Arc<Node>> {
    if page_id.value() > maximum_page {
        return Err(Error::Corrupt(
            "page reference exceeds the committed boundary".into(),
        ));
    }
    if let Some(node) = page_cache.get(page_id)? {
        return Ok(node);
    }
    let node = read_node_from_file(file, page_id, maximum_transaction_id, maximum_page)?;
    page_cache.insert(page_id, Arc::clone(&node))?;
    Ok(node)
}

fn read_node_from_file(
    file: &File,
    page_id: PageId,
    maximum_transaction_id: TransactionId,
    maximum_page: u64,
) -> Result<Arc<Node>> {
    if page_id.value() > maximum_page {
        return Err(Error::Corrupt(
            "page reference exceeds the committed boundary".into(),
        ));
    }
    let mut page = [0_u8; PAGE_SIZE];
    read_exact_at(file, &mut page, page_offset(page_id)?)?;
    Node::decode(&page, page_id, maximum_transaction_id).map(Arc::new)
}

pub(crate) fn next_page_after(file_len: u64) -> Result<u64> {
    let page_size = PAGE_SIZE as u64;
    let rounded = file_len
        .checked_add(page_size - 1)
        .ok_or(Error::FileTooLarge)?;
    Ok((rounded / page_size).max(PageId::FIRST_DATA_PAGE))
}

fn page_offset(page_id: PageId) -> Result<u64> {
    page_id
        .value()
        .checked_mul(PAGE_SIZE as u64)
        .ok_or(Error::FileTooLarge)
}

#[cfg(test)]
mod tests;
