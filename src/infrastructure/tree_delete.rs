use super::metadata::PAGE_SIZE;
use super::page::{LeafEntry, Node};
use super::tree::PageWriter;
use crate::domain::{MAX_TREE_DEPTH, PageId};
use crate::{Error, Result};

const REBALANCE_BELOW_BYTES: usize = PAGE_SIZE / 2;

struct Replacement {
    page_id: PageId,
    minimum_key: Vec<u8>,
}

struct BranchParts<'a> {
    children: &'a [PageId],
    separators: &'a [Vec<u8>],
}

struct SiblingPages {
    left: PageId,
    right: PageId,
}

enum DeleteOutcome {
    Unchanged,
    Removed,
    Replaced(Replacement),
}

pub(crate) fn delete_root(
    writer: &mut impl PageWriter,
    root: Option<PageId>,
    root_minimum: Option<&[u8]>,
    key: &[u8],
) -> Result<(Option<PageId>, Option<Vec<u8>>, bool)> {
    let Some(root) = root else {
        return Ok((None, None, false));
    };
    let root_minimum = root_minimum.ok_or(Error::InternalInvariant("root minimum is missing"))?;
    match delete_page(writer, root, root_minimum, key, 0)? {
        DeleteOutcome::Unchanged => Ok((Some(root), Some(root_minimum.to_vec()), false)),
        DeleteOutcome::Removed => Ok((None, None, true)),
        DeleteOutcome::Replaced(replacement) => Ok((
            Some(replacement.page_id),
            Some(replacement.minimum_key),
            true,
        )),
    }
}

fn delete_page(
    writer: &mut impl PageWriter,
    page_id: PageId,
    subtree_minimum: &[u8],
    key: &[u8],
    depth: usize,
) -> Result<DeleteOutcome> {
    ensure_depth(depth)?;
    match writer.read_node(page_id)?.as_ref() {
        Node::Leaf(stored_entries) => {
            delete_from_leaf(writer, page_id, stored_entries.to_vec(), key)
        }
        Node::PackedLeaf(leaf) => delete_from_leaf(writer, page_id, leaf.to_entries()?, key),
        Node::Branch {
            children: stored_children,
            separators: stored_separators,
        } => delete_from_branch(
            writer,
            page_id,
            stored_children,
            stored_separators,
            subtree_minimum,
            key,
            depth,
        ),
        Node::Overflow { .. } => Err(Error::Corrupt(
            "tree path references an overflow page".into(),
        )),
    }
}

fn delete_from_leaf(
    writer: &mut impl PageWriter,
    page_id: PageId,
    mut entries: Vec<LeafEntry>,
    key: &[u8],
) -> Result<DeleteOutcome> {
    let Ok(index) = entries.binary_search_by(|entry| entry.key.as_ref().cmp(key)) else {
        return Ok(DeleteOutcome::Unchanged);
    };
    entries.remove(index);
    if entries.is_empty() {
        return Ok(DeleteOutcome::Removed);
    }
    write_leaf_replacement(writer, page_id, entries)
}

fn delete_from_branch(
    writer: &mut impl PageWriter,
    page_id: PageId,
    stored_children: &[PageId],
    stored_separators: &[Vec<u8>],
    subtree_minimum: &[u8],
    key: &[u8],
    depth: usize,
) -> Result<DeleteOutcome> {
    let mut children = stored_children.to_vec();
    let mut separators = stored_separators.to_vec();
    let child_index = separators.partition_point(|separator| key >= separator.as_slice());
    let child_minimum = if child_index == 0 {
        subtree_minimum
    } else {
        &separators[child_index - 1]
    };
    let mut minimum_key = subtree_minimum.to_vec();
    match delete_page(writer, children[child_index], child_minimum, key, depth + 1)? {
        DeleteOutcome::Unchanged => return Ok(DeleteOutcome::Unchanged),
        DeleteOutcome::Removed => {
            if child_index == 0 {
                minimum_key = separators
                    .first()
                    .cloned()
                    .ok_or(Error::InternalInvariant("removed branch minimum"))?;
            }
            remove_child(&mut children, &mut separators, child_index);
        }
        DeleteOutcome::Replaced(replacement) => {
            children[child_index] = replacement.page_id;
            if child_index > 0 {
                separators[child_index - 1] = replacement.minimum_key;
            } else {
                minimum_key = replacement.minimum_key;
            }
        }
    }

    if children.len() > 1 {
        rebalance_child(writer, &mut children, &mut separators, child_index)?;
    }
    match children.as_slice() {
        [] => Ok(DeleteOutcome::Removed),
        [only] => Ok(DeleteOutcome::Replaced(Replacement {
            page_id: *only,
            minimum_key,
        })),
        _ => {
            let page_id = writer.replace_node(
                page_id,
                &Node::Branch {
                    children,
                    separators,
                },
            )?;
            Ok(DeleteOutcome::Replaced(Replacement {
                page_id,
                minimum_key,
            }))
        }
    }
}

fn remove_child(children: &mut Vec<PageId>, separators: &mut Vec<Vec<u8>>, index: usize) {
    children.remove(index);
    if index == 0 {
        separators.remove(0);
    } else {
        separators.remove(index - 1);
    }
}

fn rebalance_child(
    writer: &mut impl PageWriter,
    children: &mut Vec<PageId>,
    separators: &mut Vec<Vec<u8>>,
    changed_index: usize,
) -> Result<()> {
    let changed_index = changed_index.min(children.len() - 1);
    let changed = writer.read_node(children[changed_index])?;
    if changed.encoded_len() >= REBALANCE_BELOW_BYTES {
        return Ok(());
    }
    let left_index = changed_index.saturating_sub(1);
    let right_index = left_index + 1;
    let left = writer.read_node(children[left_index])?;
    let right = writer.read_node(children[right_index])?;
    let left_page = children[left_index];
    let right_page = children[right_index];

    if let (Some(left_entries), Some(right_entries)) =
        (left.leaf_entries_owned()?, right.leaf_entries_owned()?)
    {
        return rebalance_leaves(
            writer,
            children,
            separators,
            left_index,
            SiblingPages {
                left: left_page,
                right: right_page,
            },
            &left_entries,
            &right_entries,
        );
    }

    match (left.as_ref(), right.as_ref()) {
        (
            Node::Branch {
                children: left_children,
                separators: left_separators,
            },
            Node::Branch {
                children: right_children,
                separators: right_separators,
            },
        ) => rebalance_branches(
            writer,
            children,
            separators,
            left_index,
            SiblingPages {
                left: left_page,
                right: right_page,
            },
            BranchParts {
                children: left_children,
                separators: left_separators,
            },
            BranchParts {
                children: right_children,
                separators: right_separators,
            },
        ),
        _ => Err(Error::Corrupt(
            "adjacent tree children have different node kinds".into(),
        )),
    }
}

fn rebalance_leaves(
    writer: &mut impl PageWriter,
    children: &mut Vec<PageId>,
    separators: &mut Vec<Vec<u8>>,
    left_index: usize,
    pages: SiblingPages,
    left: &[LeafEntry],
    right: &[LeafEntry],
) -> Result<()> {
    let mut entries = Vec::with_capacity(left.len() + right.len());
    entries.extend_from_slice(left);
    entries.extend_from_slice(right);
    if Node::leaf_entries_fit(&entries) {
        children[left_index] = writer.replace_node(pages.left, &Node::Leaf(entries))?;
        children.remove(left_index + 1);
        separators.remove(left_index);
        return Ok(());
    }

    let split = choose_leaf_split(&entries).ok_or(Error::InternalInvariant(
        "underfull leaves could not be redistributed",
    ))?;
    let right_entries = entries.split_off(split);
    separators[left_index] = right_entries[0].key.to_vec();
    children[left_index] = writer.replace_node(pages.left, &Node::Leaf(entries))?;
    children[left_index + 1] = writer.replace_node(pages.right, &Node::Leaf(right_entries))?;
    Ok(())
}

fn rebalance_branches(
    writer: &mut impl PageWriter,
    children: &mut Vec<PageId>,
    separators: &mut Vec<Vec<u8>>,
    left_index: usize,
    pages: SiblingPages,
    left: BranchParts<'_>,
    right: BranchParts<'_>,
) -> Result<()> {
    let mut combined_children = Vec::with_capacity(left.children.len() + right.children.len());
    combined_children.extend_from_slice(left.children);
    combined_children.extend_from_slice(right.children);
    let mut combined_separators = Vec::with_capacity(combined_children.len() - 1);
    combined_separators.extend_from_slice(left.separators);
    combined_separators.push(separators[left_index].clone());
    combined_separators.extend_from_slice(right.separators);
    if Node::branch_entries_fit(&combined_children, &combined_separators) {
        children[left_index] = writer.replace_node(
            pages.left,
            &Node::Branch {
                children: combined_children,
                separators: combined_separators,
            },
        )?;
        children.remove(left_index + 1);
        separators.remove(left_index);
        return Ok(());
    }

    let split = choose_branch_split(&combined_children, &combined_separators).ok_or(
        Error::InternalInvariant("underfull branches could not be redistributed"),
    )?;
    let right_children = combined_children.split_off(split);
    let right_separators = combined_separators.split_off(split);
    let promoted = combined_separators.pop().ok_or(Error::InternalInvariant(
        "missing redistributed branch separator",
    ))?;
    children[left_index] = writer.replace_node(
        pages.left,
        &Node::Branch {
            children: combined_children,
            separators: combined_separators,
        },
    )?;
    children[left_index + 1] = writer.replace_node(
        pages.right,
        &Node::Branch {
            children: right_children,
            separators: right_separators,
        },
    )?;
    separators[left_index] = promoted;
    Ok(())
}

fn choose_leaf_split(entries: &[LeafEntry]) -> Option<usize> {
    (1..entries.len())
        .filter(|index| {
            Node::leaf_entries_fit(&entries[..*index]) && Node::leaf_entries_fit(&entries[*index..])
        })
        .min_by_key(|index| {
            Node::leaf_encoded_len(&entries[..*index])
                .abs_diff(Node::leaf_encoded_len(&entries[*index..]))
        })
}

fn choose_branch_split(children: &[PageId], separators: &[Vec<u8>]) -> Option<usize> {
    (2..children.len().saturating_sub(1))
        .filter(|index| branch_halves_fit(children, separators, *index))
        .min_by_key(|index| {
            Node::branch_encoded_len(&children[..*index], &separators[..index - 1]).abs_diff(
                Node::branch_encoded_len(&children[*index..], &separators[*index..]),
            )
        })
}

fn branch_halves_fit(children: &[PageId], separators: &[Vec<u8>], split: usize) -> bool {
    Node::branch_entries_fit(&children[..split], &separators[..split - 1])
        && Node::branch_entries_fit(&children[split..], &separators[split..])
}

fn write_leaf_replacement(
    writer: &mut impl PageWriter,
    replaced_page: PageId,
    entries: Vec<LeafEntry>,
) -> Result<DeleteOutcome> {
    let minimum_key = entries[0].key.to_vec();
    let page_id = writer.replace_node(replaced_page, &Node::Leaf(entries))?;
    Ok(DeleteOutcome::Replaced(Replacement {
        page_id,
        minimum_key,
    }))
}

fn ensure_depth(depth: usize) -> Result<()> {
    if depth >= MAX_TREE_DEPTH {
        Err(Error::TreeDepthExceeded)
    } else {
        Ok(())
    }
}
