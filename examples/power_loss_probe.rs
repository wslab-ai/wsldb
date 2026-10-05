use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use wsldb::{Database, DatabaseOptions};

const RECORD_BYTES: usize = 24;
const DEFAULT_BUFFER_BYTES: u64 = 16 * 1024;
const HEAD_KEY: &[u8] = b"__wsldb_power_loss_head";
const MARKER_PREFIX: &[u8] = b"commit/";
const MARKER_VALUE_BYTES: usize = 512;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let command = arguments.next().ok_or_else(usage)?;
    let mode = Mode::parse(&arguments.next().ok_or_else(usage)?)?;
    let database = PathBuf::from(arguments.next().ok_or_else(usage)?);
    let oracle = PathBuf::from(arguments.next().ok_or_else(usage)?);
    let limit = arguments
        .next()
        .map(|value| value.to_string_lossy().parse::<u64>())
        .transpose()?;
    if arguments.next().is_some() || command != "run" && limit.is_some() {
        return Err(usage().into());
    }

    match command.to_string_lossy().as_ref() {
        "init" => initialize(mode, &database, &oracle),
        "run" => run(mode, &database, &oracle, limit),
        "verify" => verify(mode, &database, &oracle).map(|_| ()),
        _ => Err(usage().into()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Direct,
    Buffered,
}

impl Mode {
    fn parse(value: &std::ffi::OsStr) -> Result<Self, Box<dyn Error>> {
        match value.to_string_lossy().as_ref() {
            "direct" => Ok(Self::Direct),
            "buffered" => Ok(Self::Buffered),
            _ => Err(usage().into()),
        }
    }

    const fn magic(self) -> [u8; 8] {
        match self {
            Self::Direct => *b"WSLDPWD0",
            Self::Buffered => *b"WSLDPWB0",
        }
    }

    fn options(self) -> Result<DatabaseOptions, Box<dyn Error>> {
        let bytes = match self {
            Self::Direct => 0,
            Self::Buffered => std::env::var("WSLDB_POWER_LOSS_BUFFER_BYTES")
                .ok()
                .map(|value| value.parse())
                .transpose()?
                .unwrap_or(DEFAULT_BUFFER_BYTES),
        };
        Ok(DatabaseOptions::new().write_buffer_bytes(bytes))
    }
}

fn initialize(mode: Mode, database_path: &Path, oracle_path: &Path) -> Result<(), Box<dyn Error>> {
    if database_path.exists() || oracle_path.exists() || wal_path(database_path).exists() {
        return Err(invalid("probe paths must not exist before init").into());
    }
    let database = Database::open_with_options(database_path, mode.options()?)?;
    if database.verify()?.entries != 0 {
        return Err(invalid("new probe database is not empty").into());
    }
    let mut oracle = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(oracle_path)?;
    append_oracle(&mut oracle, mode, 0)?;
    sync_parent(oracle_path)?;
    println!(
        "initialized mode={} database={} oracle={}",
        mode_name(mode),
        database_path.display(),
        oracle_path.display()
    );
    Ok(())
}

fn run(
    mode: Mode,
    database_path: &Path,
    oracle_path: &Path,
    limit: Option<u64>,
) -> Result<(), Box<dyn Error>> {
    require_paths(database_path, oracle_path)?;
    let database = Database::open_with_options(database_path, mode.options()?)?;
    let verified = verify_database(&database)?;
    let mut oracle = OpenOptions::new()
        .read(true)
        .write(true)
        .open(oracle_path)?;
    let oracle_state = read_oracle(&mut oracle, mode)?;
    ensure_recoverable(verified.logical_commits, oracle_state.last_commit)?;
    if oracle_state.trailing_bytes != 0 {
        oracle.set_len(oracle_state.valid_bytes)?;
        oracle.sync_all()?;
    }
    if verified.logical_commits == oracle_state.last_commit.saturating_add(1) {
        append_oracle(&mut oracle, mode, verified.logical_commits)?;
        println!("RECOVERED {}", verified.logical_commits);
    }

    let mut current = verified.logical_commits;
    let mut remaining = limit;
    while remaining.is_none_or(|remaining| remaining != 0) {
        let next = current
            .checked_add(1)
            .ok_or_else(|| invalid("logical commit counter is exhausted"))?;
        let mut writer = database.write()?;
        writer.put(&marker_key(next), &marker_value(next))?;
        writer.put(HEAD_KEY, &counter_value(next))?;
        let transaction_id = writer.commit()?;
        append_oracle(&mut oracle, mode, next)?;
        println!("ACK {next} transaction={}", transaction_id.value());
        io::stdout().flush()?;
        current = next;
        if let Some(left) = &mut remaining {
            *left -= 1;
        }
    }
    Ok(())
}

fn verify(
    mode: Mode,
    database_path: &Path,
    oracle_path: &Path,
) -> Result<Verified, Box<dyn Error>> {
    require_paths(database_path, oracle_path)?;
    let database = Database::open_with_options(database_path, mode.options()?)?;
    let verified = verify_database(&database)?;
    let mut oracle = File::open(oracle_path)?;
    let oracle_state = read_oracle(&mut oracle, mode)?;
    ensure_recoverable(verified.logical_commits, oracle_state.last_commit)?;
    let report = database.verify()?;
    println!(
        "verified mode={} logical_commits={} oracle_commits={} trailing_oracle_bytes={} transaction={} entries={} logical_hash={}",
        mode_name(mode),
        verified.logical_commits,
        oracle_state.last_commit,
        oracle_state.trailing_bytes,
        report.transaction_id.value(),
        report.entries,
        hex(&report.logical_hash)
    );
    Ok(verified)
}

#[derive(Clone, Copy)]
struct Verified {
    logical_commits: u64,
}

fn verify_database(database: &Database) -> Result<Verified, Box<dyn Error>> {
    let report = database.verify()?;
    let reader = database.read()?;
    let logical_commits = match reader.get(HEAD_KEY)? {
        Some(value) => decode_counter(&value)?,
        None if report.entries == 0 => 0,
        None => return Err(invalid("non-empty probe database has no logical head").into()),
    };
    let expected_entries = if logical_commits == 0 {
        0
    } else {
        logical_commits
            .checked_add(1)
            .ok_or_else(|| invalid("probe entry count is exhausted"))?
    };
    if report.entries != expected_entries {
        return Err(invalid("probe entry count does not match its logical head").into());
    }

    let mut next_marker = 1_u64;
    let mut saw_head = logical_commits == 0;
    for (key, value) in reader.iter()? {
        if key == HEAD_KEY {
            if saw_head || decode_counter(&value)? != logical_commits {
                return Err(invalid("probe contains an invalid logical head").into());
            }
            saw_head = true;
            continue;
        }
        if key != marker_key(next_marker) || value != marker_value(next_marker) {
            return Err(invalid("probe commit markers are not a complete prefix").into());
        }
        next_marker = next_marker
            .checked_add(1)
            .ok_or_else(|| invalid("probe marker counter is exhausted"))?;
    }
    if !saw_head || next_marker != logical_commits.saturating_add(1) {
        return Err(invalid("probe commit markers are incomplete").into());
    }
    Ok(Verified { logical_commits })
}

struct OracleState {
    last_commit: u64,
    valid_bytes: u64,
    trailing_bytes: u64,
}

fn read_oracle(file: &mut File, mode: Mode) -> Result<OracleState, Box<dyn Error>> {
    file.seek(SeekFrom::Start(0))?;
    let mut record = [0_u8; RECORD_BYTES];
    let mut filled = 0_usize;
    let mut last: Option<u64> = None;
    let mut valid_bytes = 0_u64;
    loop {
        let read = file.read(&mut record[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
        if filled != RECORD_BYTES {
            continue;
        }
        let commit = decode_oracle_record(&record, mode)?;
        if last.map_or(commit != 0, |previous| {
            previous.checked_add(1) != Some(commit)
        }) {
            return Err(invalid("oracle commit sequence is discontinuous").into());
        }
        last = Some(commit);
        valid_bytes = valid_bytes
            .checked_add(RECORD_BYTES as u64)
            .ok_or_else(|| invalid("oracle is too large"))?;
        filled = 0;
    }
    let last_commit = last.ok_or_else(|| invalid("oracle has no complete initial record"))?;
    Ok(OracleState {
        last_commit,
        valid_bytes,
        trailing_bytes: filled as u64,
    })
}

fn append_oracle(file: &mut File, mode: Mode, commit: u64) -> Result<(), Box<dyn Error>> {
    file.seek(SeekFrom::End(0))?;
    let mut record = [0_u8; RECORD_BYTES];
    record[..8].copy_from_slice(&mode.magic());
    record[8..16].copy_from_slice(&commit.to_le_bytes());
    record[16..24].copy_from_slice(&(!commit).to_le_bytes());
    file.write_all(&record)?;
    file.sync_all()?;
    Ok(())
}

fn decode_oracle_record(record: &[u8; RECORD_BYTES], mode: Mode) -> Result<u64, Box<dyn Error>> {
    if record[..8] != mode.magic() {
        return Err(invalid("oracle mode or magic is invalid").into());
    }
    let commit = u64::from_le_bytes(record[8..16].try_into()?);
    let complement = u64::from_le_bytes(record[16..24].try_into()?);
    if complement != !commit {
        return Err(invalid("oracle record checksum is invalid").into());
    }
    Ok(commit)
}

fn ensure_recoverable(database: u64, oracle: u64) -> Result<(), Box<dyn Error>> {
    if database == oracle || database == oracle.saturating_add(1) {
        Ok(())
    } else {
        Err(invalid("database lost an acknowledged commit or advanced by more than one").into())
    }
}

fn marker_key(commit: u64) -> Vec<u8> {
    let mut key = MARKER_PREFIX.to_vec();
    key.extend_from_slice(&commit.to_be_bytes());
    key
}

fn marker_value(commit: u64) -> Vec<u8> {
    let mut value = vec![0_u8; MARKER_VALUE_BYTES];
    let mut state = commit ^ 0x4D59_5DF4_D0F3_3173;
    for chunk in value.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let bytes = state.to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    value
}

fn counter_value(commit: u64) -> [u8; 16] {
    let mut value = [0_u8; 16];
    value[..8].copy_from_slice(&commit.to_le_bytes());
    value[8..].copy_from_slice(&(!commit).to_le_bytes());
    value
}

fn decode_counter(value: &[u8]) -> Result<u64, Box<dyn Error>> {
    if value.len() != 16 {
        return Err(invalid("logical head width is invalid").into());
    }
    let commit = u64::from_le_bytes(value[..8].try_into()?);
    let complement = u64::from_le_bytes(value[8..].try_into()?);
    if complement != !commit {
        return Err(invalid("logical head checksum is invalid").into());
    }
    Ok(commit)
}

fn require_paths(database: &Path, oracle: &Path) -> Result<(), Box<dyn Error>> {
    if !database.is_file() || !oracle.is_file() {
        return Err(invalid("probe database and oracle must already exist").into());
    }
    Ok(())
}

fn wal_path(database: &Path) -> PathBuf {
    let mut path = database.as_os_str().to_owned();
    path.push(".wal");
    path.into()
}

fn sync_parent(path: &Path) -> Result<(), Box<dyn Error>> {
    #[cfg(unix)]
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Direct => "direct",
        Mode::Buffered => "buffered",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: power_loss_probe <init|run|verify> <direct|buffered> <database> <oracle> [run-commits]",
    )
}

#[cfg(test)]
mod tests {
    use super::{Mode, counter_value, decode_counter, decode_oracle_record, ensure_recoverable};

    #[test]
    fn redundant_counters_and_oracle_records_fail_closed() {
        for commit in [0, 1, u64::MAX] {
            assert_eq!(decode_counter(&counter_value(commit)).unwrap(), commit);
            let mut record = [0_u8; 24];
            record[..8].copy_from_slice(&Mode::Direct.magic());
            record[8..16].copy_from_slice(&commit.to_le_bytes());
            record[16..].copy_from_slice(&(!commit).to_le_bytes());
            assert_eq!(decode_oracle_record(&record, Mode::Direct).unwrap(), commit);
            record[23] ^= 1;
            assert!(decode_oracle_record(&record, Mode::Direct).is_err());
        }
    }

    #[test]
    fn recovered_database_may_lead_the_durable_oracle_once() {
        assert!(ensure_recoverable(7, 7).is_ok());
        assert!(ensure_recoverable(8, 7).is_ok());
        assert!(ensure_recoverable(6, 7).is_err());
        assert!(ensure_recoverable(9, 7).is_err());
    }
}
