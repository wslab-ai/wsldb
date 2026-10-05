use crate::{Error, Result};

pub(crate) fn clone_bytes(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(bytes.len())
        .map_err(|_| Error::FileTooLarge)?;
    owned.extend_from_slice(bytes);
    Ok(owned)
}

pub(crate) fn zeroed_bytes(len: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| Error::FileTooLarge)?;
    bytes.resize(len, 0);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::zeroed_bytes;
    use crate::Error;

    #[test]
    fn zeroed_allocation_failure_is_typed() {
        assert!(matches!(zeroed_bytes(usize::MAX), Err(Error::FileTooLarge)));
    }
}
