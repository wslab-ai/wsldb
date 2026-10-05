use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{Error, Result};

#[derive(Clone)]
pub(crate) struct ChangeSet {
    values: BTreeMap<Arc<[u8]>, Option<Vec<u8>>>,
}

impl ChangeSet {
    pub(crate) const fn new() -> Self {
        Self {
            values: BTreeMap::new(),
        }
    }

    pub(crate) fn get(&self, key: &[u8]) -> Option<&Option<Vec<u8>>> {
        self.values.get(key)
    }

    pub(crate) fn insert(&mut self, key: &[u8], value: Option<Vec<u8>>) {
        self.values.insert(Arc::from(key), value);
    }

    pub(crate) fn insert_shared(&mut self, key: Arc<[u8]>, value: Option<Vec<u8>>) {
        self.values.insert(key, value);
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub(crate) fn sorted_iter(&self) -> impl Iterator<Item = (&[u8], Option<&[u8]>)> {
        self.values
            .iter()
            .map(|(key, value)| (key.as_ref(), value.as_deref()))
    }

    pub(crate) fn shared_sorted_iter(&self) -> impl Iterator<Item = (&Arc<[u8]>, Option<&[u8]>)> {
        self.values
            .iter()
            .map(|(key, value)| (key, value.as_deref()))
    }

    pub(crate) fn into_sorted_iter(self) -> impl Iterator<Item = (Arc<[u8]>, Option<Vec<u8>>)> {
        self.values.into_iter()
    }

    pub(crate) fn logical_bytes(&self) -> Result<u64> {
        self.values.iter().try_fold(0_u64, |total, (key, value)| {
            let bytes = key
                .len()
                .checked_add(value.as_ref().map_or(0, Vec::len))
                .and_then(|bytes| u64::try_from(bytes).ok())
                .ok_or(Error::FileTooLarge)?;
            total.checked_add(bytes).ok_or(Error::FileTooLarge)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ChangeSet;

    #[test]
    fn preserves_latest_values_in_key_order() {
        let mut changes = ChangeSet::new();
        changes.insert(b"third", Some(b"old".to_vec()));
        changes.insert(b"first", None);
        changes.insert(b"third", Some(b"new".to_vec()));
        changes.insert(b"second", Some(b"value".to_vec()));

        let ordered = changes
            .sorted_iter()
            .map(|(key, value)| (key.to_vec(), value.map(<[u8]>::to_vec)))
            .collect::<Vec<_>>();

        assert_eq!(
            ordered,
            vec![
                (b"first".to_vec(), None),
                (b"second".to_vec(), Some(b"value".to_vec())),
                (b"third".to_vec(), Some(b"new".to_vec())),
            ]
        );
        assert_eq!(changes.logical_bytes().unwrap(), 24);
    }
}
