use crate::domain::{MAX_KEY_BYTES, MAX_VALUE_BYTES};
use crate::{Error, Result};

pub(super) fn validate_key(key: &[u8]) -> Result<()> {
    if key.is_empty() {
        return Err(Error::InvalidInput("keys must not be empty"));
    }
    if key.len() > MAX_KEY_BYTES {
        return Err(Error::InvalidInput("key exceeds 1024 bytes"));
    }
    Ok(())
}

pub(super) fn validate_value(value: &[u8]) -> Result<()> {
    validate_value_len(value.len())
}

fn validate_value_len(len: usize) -> Result<()> {
    if len > MAX_VALUE_BYTES {
        return Err(Error::InvalidInput("value exceeds the 64 MiB limit"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_key, validate_value_len};
    use crate::{Error, MAX_KEY_BYTES, MAX_VALUE_BYTES};

    #[test]
    fn exported_admission_limits_are_inclusive() {
        let mut key = vec![b'k'; MAX_KEY_BYTES];
        assert!(validate_key(&key).is_ok());
        key.push(b'k');
        assert!(matches!(validate_key(&key), Err(Error::InvalidInput(_))));

        assert!(validate_value_len(MAX_VALUE_BYTES).is_ok());
        assert!(matches!(
            validate_value_len(MAX_VALUE_BYTES + 1),
            Err(Error::InvalidInput(_))
        ));
    }
}
