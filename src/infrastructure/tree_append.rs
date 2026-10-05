use super::page::{LeafEntry, Node};
use super::tree::{PageWriter, store_value};
use crate::domain::{ChangeSet, Head, MAX_TREE_DEPTH, PageId};
use crate::{Error, Result};

struct Child {
    page_id: PageId,
    minimum: Vec<u8>,
}

struct BranchFrame {
    page_id: PageId,
    minimum: Vec<u8>,
    children: Vec<PageId>,
    separators: Vec<Vec<u8>>,
}

/// Applies a transaction consisting only of keys beyond the current maximum.
///
/// The right spine is read once. Completed leaves and branches are packed in
/// ascending order, while untouched subtrees remain shared with the base root.
pub(crate) fn try_apply(
    writer: &mut impl PageWriter,
    base: Head,
    changes: &ChangeSet,
) -> Result<Option<(Option<PageId>, u64)>> {
    let Some(root) = base.root() else {
        return Ok(None);
    };
    if changes.sorted_iter().any(|(_, value)| value.is_none()) {
        return Ok(None);
    }
    let Some((first_key, Some(_))) = changes.sorted_iter().next() else {
        return Ok(None);
    };

    let root_minimum = minimum_key(writer, root)?;
    let (frames, leaf_page, mut entries) = right_spine(writer, root, root_minimum)?;
    let maximum = entries
        .last()
        .ok_or(Error::InternalInvariant("rightmost leaf is empty"))?
        .key
        .as_ref();
    if first_key <= maximum {
        return Ok(None);
    }

    entries
        .try_reserve(changes.len())
        .map_err(|_| Error::FileTooLarge)?;
    let mut added = 0_u64;
    for (key, value) in changes.sorted_iter() {
        let value = value.ok_or(Error::InternalInvariant("append value is missing"))?;
        entries.push(LeafEntry {
            key: key.into(),
            value: store_value(writer, value)?,
        });
        added = added.checked_add(1).ok_or(Error::FileTooLarge)?;
    }

    let mut replacements = persist_leaves(writer, leaf_page, entries)?;
    for frame in frames.into_iter().rev() {
        replacements = rebuild_rightmost_branch(writer, frame, replacements)?;
    }
    while replacements.len() > 1 {
        replacements = persist_new_level(writer, replacements)?;
    }
    let root = replacements
        .first()
        .ok_or(Error::InternalInvariant("append produced no root"))?
        .page_id;
    let entry_count = base
        .entry_count()
        .checked_add(added)
        .ok_or(Error::FileTooLarge)?;
    Ok(Some((Some(root), entry_count)))
}

fn minimum_key(reader: &impl PageWriter, mut page_id: PageId) -> Result<Vec<u8>> {
    for _ in 0..MAX_TREE_DEPTH {
        match reader.read_node(page_id)?.as_ref() {
            Node::Leaf(entries) => {
                return entries
                    .first()
                    .map(|entry| entry.key.to_vec())
                    .ok_or(Error::InternalInvariant("leaf is empty"));
            }
            Node::PackedLeaf(leaf) => return leaf.key(0),
            Node::Branch { children, .. } => {
                page_id = *children
                    .first()
                    .ok_or(Error::InternalInvariant("branch has no children"))?;
            }
            Node::Overflow { .. } => {
                return Err(Error::Corrupt(
                    "tree path references an overflow page".into(),
                ));
            }
        }
    }
    Err(Error::TreeDepthExceeded)
}

fn right_spine(
    reader: &impl PageWriter,
    root: PageId,
    root_minimum: Vec<u8>,
) -> Result<(Vec<BranchFrame>, PageId, Vec<LeafEntry>)> {
    let mut frames = Vec::new();
    let mut page_id = root;
    let mut minimum = root_minimum;
    for _ in 0..MAX_TREE_DEPTH {
        let node = reader.read_node(page_id)?;
        match node.as_ref() {
            Node::Leaf(entries) => return Ok((frames, page_id, entries.clone())),
            Node::PackedLeaf(leaf) => return Ok((frames, page_id, leaf.to_entries()?)),
            Node::Branch {
                children,
                separators,
            } => {
                let next_page = *children
                    .last()
                    .ok_or(Error::InternalInvariant("branch has no children"))?;
                let next_minimum = separators
                    .last()
                    .cloned()
                    .ok_or(Error::InternalInvariant("branch has no separator"))?;
                frames.push(BranchFrame {
                    page_id,
                    minimum,
                    children: children.clone(),
                    separators: separators.clone(),
                });
                page_id = next_page;
                minimum = next_minimum;
            }
            Node::Overflow { .. } => {
                return Err(Error::Corrupt(
                    "tree path references an overflow page".into(),
                ));
            }
        }
    }
    Err(Error::TreeDepthExceeded)
}

fn persist_leaves(
    writer: &mut impl PageWriter,
    replaced_page: PageId,
    entries: Vec<LeafEntry>,
) -> Result<Vec<Child>> {
    let mut output = Vec::new();
    let mut first = 0;
    while first < entries.len() {
        let mut end = first + 1;
        while end < entries.len() && Node::leaf_entries_fit(&entries[first..=end]) {
            end += 1;
        }
        let mut page_entries = Vec::new();
        page_entries
            .try_reserve_exact(end - first)
            .map_err(|_| Error::FileTooLarge)?;
        page_entries.extend_from_slice(&entries[first..end]);
        let minimum = page_entries[0].key.to_vec();
        let node = Node::Leaf(page_entries);
        output.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
        let page_id = if first == 0 {
            writer.replace_node(replaced_page, &node)?
        } else {
            writer.write_node(&node)?
        };
        output.push(Child { page_id, minimum });
        first = end;
    }
    Ok(output)
}

fn rebuild_rightmost_branch(
    writer: &mut impl PageWriter,
    mut frame: BranchFrame,
    replacements: Vec<Child>,
) -> Result<Vec<Child>> {
    frame
        .children
        .pop()
        .ok_or(Error::InternalInvariant("branch has no rightmost child"))?;
    frame.separators.pop().ok_or(Error::InternalInvariant(
        "branch has no rightmost separator",
    ))?;
    frame
        .children
        .try_reserve(replacements.len())
        .map_err(|_| Error::FileTooLarge)?;
    frame
        .separators
        .try_reserve(replacements.len())
        .map_err(|_| Error::FileTooLarge)?;
    frame
        .children
        .extend(replacements.iter().map(|child| child.page_id));
    frame
        .separators
        .extend(replacements.into_iter().map(|child| child.minimum));
    persist_branches(
        writer,
        Some(frame.page_id),
        frame.minimum,
        frame.children,
        frame.separators,
    )
}

fn persist_new_level(writer: &mut impl PageWriter, children: Vec<Child>) -> Result<Vec<Child>> {
    let mut page_ids = Vec::new();
    page_ids
        .try_reserve_exact(children.len())
        .map_err(|_| Error::FileTooLarge)?;
    let mut separators = Vec::new();
    separators
        .try_reserve_exact(children.len().saturating_sub(1))
        .map_err(|_| Error::FileTooLarge)?;
    let mut children = children.into_iter();
    let first = children
        .next()
        .ok_or(Error::InternalInvariant("new branch level is empty"))?;
    let minimum = first.minimum;
    page_ids.push(first.page_id);
    for child in children {
        page_ids.push(child.page_id);
        separators.push(child.minimum);
    }
    persist_branches(writer, None, minimum, page_ids, separators)
}

fn persist_branches(
    writer: &mut impl PageWriter,
    replaced_page: Option<PageId>,
    minimum: Vec<u8>,
    children: Vec<PageId>,
    separators: Vec<Vec<u8>>,
) -> Result<Vec<Child>> {
    if children.len() < 2 || separators.len() + 1 != children.len() {
        return Err(Error::InternalInvariant("append branch arity"));
    }
    let mut output = Vec::new();
    let mut first = 0;
    while first < children.len() {
        let mut end = (first + 2).min(children.len());
        while end < children.len()
            && branch_node(&children[first..=end], &separators[first..end]).fits()
        {
            end += 1;
        }
        if children.len() - end == 1 {
            end = end
                .checked_sub(1)
                .ok_or(Error::InternalInvariant("append branch partition"))?;
        }
        if end - first < 2 {
            return Err(Error::InternalInvariant("append branch fanout"));
        }
        let node = branch_node(&children[first..end], &separators[first..end - 1]);
        let group_minimum = if first == 0 {
            minimum.clone()
        } else {
            separators[first - 1].clone()
        };
        output.try_reserve(1).map_err(|_| Error::FileTooLarge)?;
        let page_id = if first == 0 {
            if let Some(page_id) = replaced_page {
                writer.replace_node(page_id, &node)?
            } else {
                writer.write_node(&node)?
            }
        } else {
            writer.write_node(&node)?
        };
        output.push(Child {
            page_id,
            minimum: group_minimum,
        });
        first = end;
    }
    Ok(output)
}

fn branch_node(children: &[PageId], separators: &[Vec<u8>]) -> Node {
    Node::Branch {
        children: children.to_vec(),
        separators: separators.to_vec(),
    }
}
