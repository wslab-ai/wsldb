use std::error::Error;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use wsldb::{Database, DatabaseOptions, Error as DatabaseError};

const WRITE_BUFFER_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CONCURRENT_COMMITS: u64 = 10_000;

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::var_os("WSLDB_RUN_COMPACTION_BENCH").is_none() {
        eprintln!(
            "set WSLDB_RUN_COMPACTION_BENCH=1 and WSLDB_COMPACTION_DATABASE=/existing/database.wsldb"
        );
        return Ok(());
    }
    let path = std::env::var_os("WSLDB_COMPACTION_DATABASE")
        .map(PathBuf::from)
        .ok_or("WSLDB_COMPACTION_DATABASE is required")?;
    let database = Database::open_with_options(
        &path,
        DatabaseOptions::new().write_buffer_bytes(WRITE_BUFFER_BYTES),
    )?;
    let initial = database.verify()?;
    if initial.entries == 0 {
        return Err("compaction benchmark requires a non-empty database".into());
    }
    let mut seed = database.write()?;
    seed.put(&benchmark_key(0), &benchmark_value(u64::MAX))?;
    seed.commit()?;
    let before = database.verify()?;
    let bytes_before = storage_bytes(&path)?;
    let compact_path = path.clone();
    let compactor = thread::spawn(move || {
        let started = Instant::now();
        let report = Database::compact_file(compact_path);
        (started.elapsed(), report)
    });
    wait_for_snapshot_capture(&path, &compactor)?;

    let mut latencies = Vec::new();
    let mut commits = 0_u64;
    let mut conflicts = 0_u64;
    while !compactor.is_finished() && commits < MAX_CONCURRENT_COMMITS {
        let index = commits % before.entries;
        let key = benchmark_key(index);
        let value = benchmark_value(commits);
        let started = Instant::now();
        let mut writer = database.write()?;
        writer.put(&key, &value)?;
        match writer.commit() {
            Ok(_) => {
                latencies.push(started.elapsed());
                commits += 1;
            }
            Err(DatabaseError::WriteConflict) => {
                conflicts += 1;
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let (compaction_elapsed, report) = compactor
        .join()
        .map_err(|_| "compaction benchmark worker panicked")?;
    let report = report?;
    database.refresh()?;
    let after = database.verify()?;
    if after.transaction_id != report.compacted_snapshot.transaction_id
        || after.entries != before.entries
        || after.logical_hash != report.compacted_snapshot.logical_hash
    {
        return Err("compaction result does not match the reopened database".into());
    }
    let bytes_after = storage_bytes(&path)?;
    latencies.sort_unstable();
    let p99 = percentile(&latencies, 99, 100);
    let maximum = latencies.last().copied().unwrap_or(Duration::ZERO);
    println!(
        "commits,conflicts,compaction_ns,publication_pause_ns,commit_p99_ns,commit_max_ns,bytes_before,bytes_after,source_transaction,final_transaction"
    );
    println!(
        "{commits},{conflicts},{},{},{},{},{bytes_before},{bytes_after},{},{}",
        compaction_elapsed.as_nanos(),
        report.publication_pause.as_nanos(),
        p99.as_nanos(),
        maximum.as_nanos(),
        report.source_transaction_id.value(),
        after.transaction_id.value(),
    );
    Ok(())
}

fn wait_for_snapshot_capture(
    path: &Path,
    compactor: &thread::JoinHandle<(Duration, wsldb::Result<wsldb::CompactionReport>)>,
) -> Result<(), Box<dyn Error>> {
    let file_name = path
        .file_name()
        .ok_or("database path must have a file name")?
        .to_string_lossy();
    let prefix = format!("{file_name}.wsldb-replacement-{}-", std::process::id());
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let started = Instant::now();
    loop {
        let captured = std::fs::read_dir(parent)?
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with(&prefix));
        if captured {
            return Ok(());
        }
        if compactor.is_finished() {
            return Err("compaction finished before concurrent churn began".into());
        }
        if started.elapsed() >= Duration::from_secs(5) {
            return Err("timed out waiting for compaction snapshot capture".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn storage_bytes(path: &Path) -> Result<u64, Box<dyn Error>> {
    let main = path.metadata()?.len();
    let mut wal = path.as_os_str().to_owned();
    wal.push(".wal");
    let wal = match PathBuf::from(wal).metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    main.checked_add(wal)
        .ok_or_else(|| "storage size overflow".into())
}

fn benchmark_key(index: u64) -> [u8; 16] {
    let mut key = [b'0'; 16];
    key[..8].copy_from_slice(&index.to_be_bytes());
    key
}

fn benchmark_value(mut seed: u64) -> [u8; 100] {
    let mut value = [0_u8; 100];
    for chunk in value.chunks_mut(8) {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let bytes = seed.to_le_bytes();
        let len = chunk.len();
        chunk.copy_from_slice(&bytes[..len]);
    }
    value
}

fn percentile(samples: &[Duration], numerator: usize, denominator: usize) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let index = samples
        .len()
        .saturating_mul(numerator)
        .div_ceil(denominator)
        .saturating_sub(1)
        .min(samples.len() - 1);
    samples[index]
}
