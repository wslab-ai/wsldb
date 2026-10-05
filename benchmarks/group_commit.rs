use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use wsldb::{Database, DatabaseOptions, WriteBatch};

const WRITE_BUFFER_BYTES: u64 = 1024 * 1024 * 1024;
const VALUE_BYTES: usize = 100;

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::var_os("WSLDB_RUN_GROUP_BENCH").is_none() {
        eprintln!(
            "set WSLDB_RUN_GROUP_BENCH=1 to run: cargo bench --bench group_commit -- \
             [transactions] [group_size]"
        );
        return Ok(());
    }
    let mut arguments = std::env::args().skip(1);
    let transactions = argument(&mut arguments, 4_096)?;
    let group_size = argument(&mut arguments, 16)?;
    if transactions == 0 || group_size == 0 {
        return Err("transactions and group size must be positive".into());
    }
    let directory = std::env::var_os("WSLDB_GROUP_BENCH_DIRECTORY")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    if let Some(mode) = std::env::var_os("WSLDB_GROUP_BENCH_ONLY") {
        if mode != "automatic" {
            return Err("WSLDB_GROUP_BENCH_ONLY must be automatic".into());
        }
        let path = database_path(&directory, "automatic");
        ensure_absent(&path)?;
        let automatic = run_automatic(&path, transactions, group_size)?;
        if automatic.transaction_id != transactions {
            return Err("transaction count does not match the final transaction ID".into());
        }
        report_header();
        report("automatic", transactions, group_size, automatic);
        if std::env::var_os("WSLDB_GROUP_BENCH_KEEP").is_none() {
            remove_database(&path)?;
        }
        return Ok(());
    }
    let sequence_path = database_path(&directory, "sequential");
    let grouped_path = database_path(&directory, "grouped");
    let automatic_path = database_path(&directory, "automatic");
    ensure_absent(&sequence_path)?;
    ensure_absent(&grouped_path)?;
    ensure_absent(&automatic_path)?;
    let order =
        std::env::var("WSLDB_GROUP_BENCH_ORDER").unwrap_or_else(|_| "sequential-first".to_owned());
    let (sequential, grouped, automatic) = match order.as_str() {
        "sequential-first" => {
            let sequential = run_sequential(&sequence_path, transactions)?;
            let automatic = run_automatic(&automatic_path, transactions, group_size)?;
            let grouped = run_grouped(&grouped_path, transactions, group_size)?;
            (sequential, grouped, automatic)
        }
        "grouped-first" => {
            let grouped = run_grouped(&grouped_path, transactions, group_size)?;
            let automatic = run_automatic(&automatic_path, transactions, group_size)?;
            let sequential = run_sequential(&sequence_path, transactions)?;
            (sequential, grouped, automatic)
        }
        _ => return Err("group benchmark order must be sequential-first or grouped-first".into()),
    };
    eprintln!("benchmark_order={order}");
    if sequential.logical_hash != grouped.logical_hash
        || sequential.logical_hash != automatic.logical_hash
    {
        return Err("group benchmark logical hashes differ".into());
    }
    if sequential.transaction_id != transactions
        || grouped.transaction_id != transactions
        || automatic.transaction_id != transactions
    {
        return Err("transaction count does not match the final transaction ID".into());
    }

    report_header();
    report("sequential", transactions, 1, sequential);
    report("grouped", transactions, group_size, grouped);
    report("automatic", transactions, group_size, automatic);
    if std::env::var_os("WSLDB_GROUP_BENCH_KEEP").is_none() {
        remove_database(&sequence_path)?;
        remove_database(&grouped_path)?;
        remove_database(&automatic_path)?;
    }
    Ok(())
}

fn report_header() {
    println!(
        "mode,transactions,group_size,elapsed_ns,transactions_per_second,wal_syncs,wal_bytes,logical_hash"
    );
}

struct Measurement {
    elapsed: Duration,
    transaction_id: u64,
    wal_syncs: u64,
    wal_bytes: u64,
    logical_hash: [u8; 32],
}

fn run_sequential(path: &Path, transactions: u64) -> Result<Measurement, Box<dyn Error>> {
    let database = open(path)?;
    let started = Instant::now();
    for transaction in 0..transactions {
        let mut writer = database.write()?;
        writer.put(&key(transaction), &value(transaction))?;
        writer.commit()?;
    }
    measure(database, path, started.elapsed())
}

fn run_grouped(
    path: &Path,
    transactions: u64,
    group_size: u64,
) -> Result<Measurement, Box<dyn Error>> {
    let database = open(path)?;
    let started = Instant::now();
    let mut next = 0_u64;
    while next < transactions {
        let end = next.saturating_add(group_size).min(transactions);
        let mut batches = Vec::with_capacity(usize::try_from(end - next)?);
        for transaction in next..end {
            let mut batch = WriteBatch::new();
            batch.put(&key(transaction), &value(transaction))?;
            batches.push(batch);
        }
        database.commit_group(batches)?;
        next = end;
    }
    measure(database, path, started.elapsed())
}

fn run_automatic(
    path: &Path,
    transactions: u64,
    threads: u64,
) -> Result<Measurement, Box<dyn Error>> {
    let database = open(path)?;
    let next = AtomicU64::new(0);
    let started = Instant::now();
    std::thread::scope(|scope| -> Result<(), Box<dyn Error>> {
        let mut workers = Vec::with_capacity(usize::try_from(threads)?);
        for _ in 0..threads {
            let database = database.clone();
            let next = &next;
            workers.push(scope.spawn(move || -> wsldb::Result<()> {
                loop {
                    let transaction = next.fetch_add(1, Ordering::Relaxed);
                    if transaction >= transactions {
                        return Ok(());
                    }
                    let mut batch = WriteBatch::new();
                    batch.put(&key(transaction), &value(transaction))?;
                    database.commit_batch(batch)?;
                }
            }));
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| "automatic group worker panicked")??;
        }
        Ok(())
    })?;
    measure(database, path, started.elapsed())
}

fn open(path: &Path) -> Result<Database, Box<dyn Error>> {
    Ok(Database::open_with_options(
        path,
        DatabaseOptions::new()
            .page_cache_capacity(0)
            .write_buffer_bytes(WRITE_BUFFER_BYTES),
    )?)
}

fn measure(
    database: Database,
    path: &Path,
    elapsed: Duration,
) -> Result<Measurement, Box<dyn Error>> {
    let transaction_id = database.stats()?.transaction_id.value();
    let operations = database.operational_stats()?;
    let verified = database.verify()?;
    let wal_bytes = std::fs::metadata(sidecar(path, ".wal"))?.len();
    Ok(Measurement {
        elapsed,
        transaction_id,
        wal_syncs: operations.wal_syncs.operations,
        wal_bytes,
        logical_hash: verified.logical_hash,
    })
}

fn report(mode: &str, transactions: u64, group_size: u64, result: Measurement) {
    let throughput = transactions as f64 / result.elapsed.as_secs_f64();
    println!(
        "{mode},{transactions},{group_size},{},{throughput:.3},{},{},{}",
        result.elapsed.as_nanos(),
        result.wal_syncs,
        result.wal_bytes,
        hex(&result.logical_hash),
    );
}

fn key(index: u64) -> [u8; 16] {
    let mut key = [b'0'; 16];
    key[..8].copy_from_slice(&index.to_be_bytes());
    key
}

fn value(index: u64) -> [u8; VALUE_BYTES] {
    let mut value = [0_u8; VALUE_BYTES];
    let seed = index.to_le_bytes();
    for (position, byte) in value.iter_mut().enumerate() {
        *byte = seed[position % seed.len()] ^ (position as u8).wrapping_mul(31);
    }
    value
}

fn argument(
    arguments: &mut impl Iterator<Item = String>,
    default: u64,
) -> Result<u64, Box<dyn Error>> {
    arguments
        .next()
        .map_or(Ok(default), |value| Ok(value.parse()?))
}

fn database_path(directory: &Path, mode: &str) -> PathBuf {
    directory.join(format!(
        "wsldb-group-bench-{}-{mode}.wsldb",
        std::process::id()
    ))
}

fn ensure_absent(path: &Path) -> Result<(), Box<dyn Error>> {
    for candidate in [
        path.to_path_buf(),
        sidecar(path, ".lock"),
        sidecar(path, ".wal"),
    ] {
        if candidate.exists() {
            return Err(format!("benchmark path already exists: {}", candidate.display()).into());
        }
    }
    Ok(())
}

fn remove_database(path: &Path) -> Result<(), Box<dyn Error>> {
    for candidate in [
        path.to_path_buf(),
        sidecar(path, ".lock"),
        sidecar(path, ".wal"),
    ] {
        std::fs::remove_file(candidate)?;
    }
    Ok(())
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}
