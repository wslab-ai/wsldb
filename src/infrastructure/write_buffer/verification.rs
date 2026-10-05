use super::{WriteBuffer, next_visible};
use crate::infrastructure::tree::PageReader;
use crate::infrastructure::tree_verify::{self, LogicalSnapshot};
use crate::{Result, VerificationReport};

pub(super) fn verify(
    buffer: &WriteBuffer,
    reader: &impl PageReader,
    snapshot: crate::domain::Head,
) -> Result<VerificationReport> {
    buffer.validate_snapshot(snapshot)?;
    let arena = buffer.arena.read()?;
    let mut buffered = buffer.versions.iter();
    let mut next = next_visible(&mut buffered, &arena, snapshot.transaction_id())?;
    let mut logical = LogicalSnapshot::new();

    let base =
        tree_verify::verify_with_entries(reader, buffer.checkpoint, |base_key, base_value| {
            loop {
                let Some((key, version)) = next else {
                    break;
                };
                match key.as_ref().cmp(base_key) {
                    std::cmp::Ordering::Less => {
                        if let Some(value) = arena.value(version)? {
                            logical.push(key, value)?;
                        }
                        next = next_visible(&mut buffered, &arena, snapshot.transaction_id())?;
                    }
                    std::cmp::Ordering::Equal => {
                        if let Some(value) = arena.value(version)? {
                            logical.push(key, value)?;
                        }
                        next = next_visible(&mut buffered, &arena, snapshot.transaction_id())?;
                        return Ok(());
                    }
                    std::cmp::Ordering::Greater => break,
                }
            }
            logical.push(base_key, &base_value)
        })?;

    if let Some((key, version)) = next
        && let Some(value) = arena.value(version)?
    {
        logical.push(key, value)?;
    }
    while let Some((key, version)) = next_visible(&mut buffered, &arena, snapshot.transaction_id())?
    {
        if let Some(value) = arena.value(version)? {
            logical.push(key, value)?;
        }
    }
    logical.finish(snapshot, base)
}
