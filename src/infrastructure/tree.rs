use super::packed_leaf::PackedValue;
use super::page::{LeafEntry, LeafValue, Node};
use super::tree_delete;
use crate::domain::{ChangeSet, Head, MAX_TREE_DEPTH, PageId};
use crate::{Error, Result};

pub(crate) use super::page_access::{PageReader, PageWriter};
pub(crate) use super::tree_value::{read_value, store_value};
pub(crate) use super::tree_verify::verify;
enum PointRead {
    Next(PageId),
    Value(Vec<u8>),
    Overflow(LeafValue),
    Missing,
}
pub(crate) fn get(
    reader: &impl PageReader,
    root: Option<PageId>,
    key: &[u8],
) -> Result<Option<Vec<u8>>> {
    let Some(mut page_id) = root else {
        return Ok(None);
    };

    for _ in 0..MAX_TREE_DEPTH {
        let action = reader.inspect_node(page_id, |node| match node {
            Node::Leaf(entries) => {
                let last = entries
                    .last()
                    .ok_or(Error::InternalInvariant("leaf is empty"))?;
                let entry = match key.cmp(last.key.as_ref()) {
                    std::cmp::Ordering::Greater => return Ok(PointRead::Missing),
                    std::cmp::Ordering::Equal => last,
                    std::cmp::Ordering::Less => {
                        let Ok(index) =
                            entries.binary_search_by(|entry| entry.key.as_ref().cmp(key))
                        else {
                            return Ok(PointRead::Missing);
                        };
                        &entries[index]
                    }
                };
                match &entry.value {
                    LeafValue::Inline(value) => Ok(PointRead::Value(value.to_vec())),
                    value @ LeafValue::Overflow { .. } => Ok(PointRead::Overflow(value.clone())),
                }
            }
            Node::PackedLeaf(leaf) => {
                let Ok(index) = leaf.find(key)? else {
                    return Ok(PointRead::Missing);
                };
                match leaf.value(index)? {
                    PackedValue::Inline(value) => Ok(PointRead::Value(value.to_vec())),
                    PackedValue::Overflow {
                        head,
                        len,
                        checksum,
                    } => Ok(PointRead::Overflow(LeafValue::Overflow {
                        head,
                        len,
                        checksum,
                    })),
                }
            }
            Node::Branch {
                children,
                separators,
            } => {
                let index = separators.partition_point(|separator| key >= separator.as_slice());
                Ok(PointRead::Next(children[index]))
            }
            Node::Overflow { .. } => Err(Error::Corrupt(
                "tree path references an overflow page".into(),
            )),
        })?;
        match action {
            PointRead::Next(next) => page_id = next,
            PointRead::Value(value) => return Ok(Some(value)),
            PointRead::Overflow(value) => return read_value(reader, &value).map(Some),
            PointRead::Missing => return Ok(None),
        }
    }
    Err(Error::TreeDepthExceeded)
}

pub(crate) fn apply(
    writer: &mut impl PageWriter,
    base: Head,
    changes: &ChangeSet,
) -> Result<(Option<PageId>, u64)> {
    if base.root().is_none() && changes.sorted_iter().all(|(_, value)| value.is_some()) {
        return super::bulk::load(writer, changes);
    }

    if let Some(result) = super::tree_append::try_apply(writer, base, changes)? {
        return Ok(result);
    }

    let mut root = base.root();
    let mut root_minimum = root
        .map(|page_id| minimum_key(writer, page_id))
        .transpose()?;
    let mut entry_count = base.entry_count();

    for (key, value) in changes.sorted_iter() {
        match value {
            Some(value) => {
                let (new_root, new_minimum, inserted) =
                    insert_root(writer, root, root_minimum.as_deref(), key, value)?;
                root = Some(new_root);
                root_minimum = Some(new_minimum);
                if inserted {
                    entry_count = entry_count.checked_add(1).ok_or(Error::FileTooLarge)?;
                }
            }
            None => {
                let (new_root, new_minimum, deleted) =
                    tree_delete::delete_root(writer, root, root_minimum.as_deref(), key)?;
                root = new_root;
                root_minimum = new_minimum;
                if deleted {
                    entry_count = entry_count
                        .checked_sub(1)
                        .ok_or(Error::InternalInvariant("entry count underflow"))?;
                }
            }
        }
    }

    if root.is_none() != (entry_count == 0) {
        return Err(Error::InternalInvariant("root and entry count disagree"));
    }
    Ok((root, entry_count))
}

#[derive(Debug)]
struct Replacement {
    page_id: PageId,
    minimum_key: Vec<u8>,
}

#[derive(Debug)]
enum InsertOutcome {
    One(Replacement),
    Split {
        left: Replacement,
        right: Replacement,
    },
}

#[derive(Clone, Copy)]
enum LeafSplitBias {
    Balanced,
    Prepend,
    Append,
}

#[derive(Clone, Copy)]
struct TreeEdge {
    leftmost: bool,
    rightmost: bool,
}

fn insert_root(
    writer: &mut impl PageWriter,
    root: Option<PageId>,
    root_minimum: Option<&[u8]>,
    key: &[u8],
    value: &[u8],
) -> Result<(PageId, Vec<u8>, bool)> {
    let Some(root) = root else {
        let stored_value = store_value(writer, value)?;
        let page_id = writer.write_node(&Node::Leaf(vec![LeafEntry {
            key: key.into(),
            value: stored_value,
        }]))?;
        return Ok((page_id, key.to_vec(), true));
    };
    let root_minimum = root_minimum.ok_or(Error::InternalInvariant("root minimum is missing"))?;

    let (outcome, inserted) = insert_page(
        writer,
        root,
        root_minimum,
        key,
        value,
        0,
        TreeEdge {
            leftmost: true,
            rightmost: true,
        },
    )?;
    let (page_id, minimum_key) = match outcome {
        InsertOutcome::One(replacement) => (replacement.page_id, replacement.minimum_key),
        InsertOutcome::Split { left, right } => {
            let minimum_key = left.minimum_key;
            let page_id = writer.write_node(&Node::Branch {
                children: vec![left.page_id, right.page_id],
                separators: vec![right.minimum_key],
            })?;
            (page_id, minimum_key)
        }
    };
    Ok((page_id, minimum_key, inserted))
}

fn insert_page(
    writer: &mut impl PageWriter,
    page_id: PageId,
    subtree_minimum: &[u8],
    key: &[u8],
    value: &[u8],
    depth: usize,
    edge: TreeEdge,
) -> Result<(InsertOutcome, bool)> {
    ensure_depth(depth)?;
    match writer.read_node(page_id)?.as_ref() {
        Node::Leaf(stored_entries) => {
            insert_leaf_entries(writer, page_id, stored_entries.clone(), key, value, edge)
        }
        Node::PackedLeaf(leaf) => {
            insert_leaf_entries(writer, page_id, leaf.to_entries()?, key, value, edge)
        }
        Node::Branch {
            children: stored_children,
            separators: stored_separators,
        } => {
            let mut children = stored_children.clone();
            let mut separators = stored_separators.clone();
            let child_index = separators.partition_point(|separator| key >= separator.as_slice());
            let child_edge = TreeEdge {
                leftmost: edge.leftmost && child_index == 0,
                rightmost: edge.rightmost && child_index + 1 == children.len(),
            };
            let child_minimum = if child_index == 0 {
                subtree_minimum
            } else {
                &separators[child_index - 1]
            };
            let (outcome, inserted) = insert_page(
                writer,
                children[child_index],
                child_minimum,
                key,
                value,
                depth + 1,
                child_edge,
            )?;
            let minimum_key = match outcome {
                InsertOutcome::One(replacement) => {
                    children[child_index] = replacement.page_id;
                    if child_index > 0 {
                        separators[child_index - 1] = replacement.minimum_key.clone();
                        subtree_minimum.to_vec()
                    } else {
                        replacement.minimum_key
                    }
                }
                InsertOutcome::Split { left, right } => {
                    children[child_index] = left.page_id;
                    if child_index > 0 {
                        separators[child_index - 1] = left.minimum_key.clone();
                    }
                    children.insert(child_index + 1, right.page_id);
                    separators.insert(child_index, right.minimum_key);
                    if child_index == 0 {
                        left.minimum_key
                    } else {
                        subtree_minimum.to_vec()
                    }
                }
            };
            Ok((
                persist_branch(writer, page_id, children, separators, minimum_key)?,
                inserted,
            ))
        }
        Node::Overflow { .. } => Err(Error::Corrupt(
            "tree path references an overflow page".into(),
        )),
    }
}

fn insert_leaf_entries(
    writer: &mut impl PageWriter,
    page_id: PageId,
    mut entries: Vec<LeafEntry>,
    key: &[u8],
    value: &[u8],
    edge: TreeEdge,
) -> Result<(InsertOutcome, bool)> {
    let stored_value = store_value(writer, value)?;
    let (split_bias, inserted) = match entries.binary_search_by(|entry| entry.key.as_ref().cmp(key))
    {
        Ok(index) => {
            entries[index].value = stored_value;
            (LeafSplitBias::Balanced, false)
        }
        Err(index) => {
            let split_bias = if index == 0 && edge.leftmost {
                LeafSplitBias::Prepend
            } else if index == entries.len() && edge.rightmost {
                LeafSplitBias::Append
            } else {
                LeafSplitBias::Balanced
            };
            entries.insert(
                index,
                LeafEntry {
                    key: key.into(),
                    value: stored_value,
                },
            );
            (split_bias, true)
        }
    };
    Ok((
        persist_leaf(writer, page_id, entries, split_bias)?,
        inserted,
    ))
}

fn persist_leaf(
    writer: &mut impl PageWriter,
    replaced_page: PageId,
    entries: Vec<LeafEntry>,
    split_bias: LeafSplitBias,
) -> Result<InsertOutcome> {
    if Node::leaf_entries_fit(&entries) {
        let minimum_key = entries[0].key.to_vec();
        return Ok(InsertOutcome::One(Replacement {
            page_id: writer.replace_node(replaced_page, &Node::Leaf(entries))?,
            minimum_key,
        }));
    }

    let split = choose_leaf_split(&entries, split_bias).ok_or(Error::InternalInvariant(
        "leaf could not be split into fitting pages",
    ))?;
    let left_entries = entries[..split].to_vec();
    let right_entries = entries[split..].to_vec();
    let left_minimum = left_entries[0].key.to_vec();
    let right_minimum = right_entries[0].key.to_vec();
    let left_page = writer.replace_node(replaced_page, &Node::Leaf(left_entries))?;
    let right_page = writer.write_hot_node(&Node::Leaf(right_entries))?;
    Ok(InsertOutcome::Split {
        left: Replacement {
            page_id: left_page,
            minimum_key: left_minimum,
        },
        right: Replacement {
            page_id: right_page,
            minimum_key: right_minimum,
        },
    })
}

fn persist_branch(
    writer: &mut impl PageWriter,
    replaced_page: PageId,
    mut children: Vec<PageId>,
    mut separators: Vec<Vec<u8>>,
    minimum_key: Vec<u8>,
) -> Result<InsertOutcome> {
    if Node::branch_entries_fit(&children, &separators) {
        return Ok(InsertOutcome::One(Replacement {
            page_id: writer.replace_node(
                replaced_page,
                &Node::Branch {
                    children,
                    separators,
                },
            )?,
            minimum_key,
        }));
    }

    let split = choose_branch_split(&children, &separators).ok_or(Error::InternalInvariant(
        "branch could not be split into fitting pages",
    ))?;
    let right_children = children.split_off(split);
    let right_separators = separators.split_off(split);
    let right_minimum = separators.pop().ok_or(Error::InternalInvariant(
        "missing promoted branch separator",
    ))?;
    let left_page = writer.replace_node(
        replaced_page,
        &Node::Branch {
            children,
            separators,
        },
    )?;
    let right_page = writer.write_node(&Node::Branch {
        children: right_children,
        separators: right_separators,
    })?;
    Ok(InsertOutcome::Split {
        left: Replacement {
            page_id: left_page,
            minimum_key,
        },
        right: Replacement {
            page_id: right_page,
            minimum_key: right_minimum,
        },
    })
}

fn choose_leaf_split(entries: &[LeafEntry], bias: LeafSplitBias) -> Option<usize> {
    let fitting = || {
        (1..entries.len()).filter(|index| {
            Node::leaf_entries_fit(&entries[..*index]) && Node::leaf_entries_fit(&entries[*index..])
        })
    };
    match bias {
        LeafSplitBias::Balanced => fitting().min_by_key(|index| {
            Node::leaf_encoded_len(&entries[..*index])
                .abs_diff(Node::leaf_encoded_len(&entries[*index..]))
        }),
        LeafSplitBias::Prepend => fitting().min(),
        LeafSplitBias::Append => fitting().max(),
    }
}

fn choose_branch_split(children: &[PageId], separators: &[Vec<u8>]) -> Option<usize> {
    (2..children.len().saturating_sub(1))
        .filter(|index| {
            Node::branch_entries_fit(&children[..*index], &separators[..index - 1])
                && Node::branch_entries_fit(&children[*index..], &separators[*index..])
        })
        .min_by_key(|index| {
            Node::branch_encoded_len(&children[..*index], &separators[..index - 1]).abs_diff(
                Node::branch_encoded_len(&children[*index..], &separators[*index..]),
            )
        })
}

fn minimum_key(reader: &impl PageReader, mut page_id: PageId) -> Result<Vec<u8>> {
    for _ in 0..MAX_TREE_DEPTH {
        match reader.read_node(page_id)?.as_ref() {
            Node::Leaf(entries) => return Ok(entries[0].key.to_vec()),
            Node::PackedLeaf(leaf) => return leaf.key(0),
            Node::Branch { children, .. } => page_id = children[0],
            Node::Overflow { .. } => {
                return Err(Error::Corrupt(
                    "tree path references an overflow page".into(),
                ));
            }
        }
    }
    Err(Error::TreeDepthExceeded)
}

fn ensure_depth(depth: usize) -> Result<()> {
    if depth >= MAX_TREE_DEPTH {
        Err(Error::TreeDepthExceeded)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
