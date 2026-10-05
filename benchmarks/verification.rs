use std::error::Error;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use wsldb::{Database, DatabaseOptions, VerificationReport};

const DEFAULT_MEASURED_RUNS: u32 = 5;
const DEFAULT_WARMUP_RUNS: u32 = 1;
const DEFAULT_CACHE_PAGES: usize = 65_536;

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::var_os("WSLDB_RUN_VERIFY_BENCH").is_none() {
        eprintln!(
            "set WSLDB_RUN_VERIFY_BENCH=1 and WSLDB_VERIFY_DATABASE=/existing/database.wsldb"
        );
        return Ok(());
    }

    let path = required_path("WSLDB_VERIFY_DATABASE")?;
    if !path.is_file() {
        return Err("WSLDB_VERIFY_DATABASE must name an existing regular file".into());
    }
    let measured = configured("WSLDB_VERIFY_RUNS", DEFAULT_MEASURED_RUNS)?;
    let warmups = configured("WSLDB_VERIFY_WARMUP_RUNS", DEFAULT_WARMUP_RUNS)?;
    let cache_pages = configured("WSLDB_VERIFY_CACHE_PAGES", DEFAULT_CACHE_PAGES)?;
    if measured == 0 || warmups == 0 {
        return Err("WSLDB_VERIFY_RUNS and WSLDB_VERIFY_WARMUP_RUNS must be non-zero".into());
    }

    eprintln!(
        "verification_database={} warmups={warmups} measured={measured} cache_pages={cache_pages}",
        path.display()
    );
    println!(
        "metric,run,elapsed_ns,transaction_id,entries,tree_pages,overflow_pages,tree_depth,logical_hash"
    );

    let expected = run_once(&path, cache_pages, 0, false)?;
    for warmup in 1..warmups {
        require_same(expected, run_once(&path, cache_pages, warmup, false)?)?;
    }
    for run in 1..=measured {
        let actual = run_once(&path, cache_pages, run, true)?;
        require_same(expected, actual)?;
    }
    Ok(())
}

fn run_once(
    path: &Path,
    cache_pages: usize,
    run: u32,
    report: bool,
) -> Result<VerificationReport, Box<dyn Error>> {
    let database = Database::open_with_options(
        path,
        DatabaseOptions::new().page_cache_capacity(cache_pages),
    )?;
    let started = Instant::now();
    let verification = black_box(database.verify()?);
    let elapsed = started.elapsed();
    if report {
        println!(
            "w7_full_verification,{run},{},{},{},{},{},{},{}",
            elapsed.as_nanos(),
            verification.transaction_id.value(),
            verification.entries,
            verification.tree_pages,
            verification.overflow_pages,
            verification.tree_depth,
            hex(&verification.logical_hash),
        );
    }
    Ok(verification)
}

fn require_same(
    expected: VerificationReport,
    actual: VerificationReport,
) -> Result<(), Box<dyn Error>> {
    if actual != expected {
        return Err("verification result changed between benchmark runs".into());
    }
    Ok(())
}

fn required_path(name: &str) -> Result<std::path::PathBuf, Box<dyn Error>> {
    std::env::var_os(name)
        .map(std::path::PathBuf::from)
        .ok_or_else(|| format!("{name} is required").into())
}

fn configured<T>(name: &str, default: T) -> Result<T, Box<dyn Error>>
where
    T: std::str::FromStr,
    T::Err: Error + 'static,
{
    std::env::var(name).map_or(Ok(default), |value| {
        value
            .parse::<T>()
            .map_err(|error| Box::new(error) as Box<dyn Error>)
    })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}
