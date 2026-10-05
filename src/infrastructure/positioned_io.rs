use std::fs::File;
use std::io::{self, ErrorKind};

use super::fault_injection;

#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;

pub(crate) fn read_exact_at(file: &File, mut bytes: &mut [u8], mut offset: u64) -> io::Result<()> {
    while !bytes.is_empty() {
        #[cfg(unix)]
        let result = file.read_at(bytes, offset);
        #[cfg(windows)]
        let result = file.seek_read(bytes, offset);

        match result {
            Ok(0) => return Err(io::Error::from(ErrorKind::UnexpectedEof)),
            Ok(read) => {
                let read = u64::try_from(read)
                    .map_err(|_| io::Error::other("read length does not fit in u64"))?;
                offset = offset
                    .checked_add(read)
                    .ok_or_else(|| io::Error::other("read offset overflow"))?;
                bytes = &mut bytes[usize::try_from(read)
                    .map_err(|_| io::Error::other("read length does not fit in usize"))?..];
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) fn write_all_at(file: &File, mut bytes: &[u8], mut offset: u64) -> io::Result<()> {
    while !bytes.is_empty() {
        let write_len = fault_injection::positioned_write_len(bytes.len())?;
        let chunk = &bytes[..write_len];
        #[cfg(unix)]
        let result = file.write_at(chunk, offset);
        #[cfg(windows)]
        let result = file.seek_write(chunk, offset);

        match result {
            Ok(0) => return Err(io::Error::from(ErrorKind::WriteZero)),
            Ok(written) => {
                fault_injection::record_positioned_write(written);
                let written = u64::try_from(written)
                    .map_err(|_| io::Error::other("write length does not fit in u64"))?;
                offset = offset
                    .checked_add(written)
                    .ok_or_else(|| io::Error::other("write offset overflow"))?;
                bytes = &bytes[usize::try_from(written)
                    .map_err(|_| io::Error::other("write length does not fit in usize"))?..];
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
