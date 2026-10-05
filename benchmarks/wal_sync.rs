use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RECORD_BYTES: usize = 256;
const DEFAULT_RECORDS: usize = 64;
const DEFAULT_RUNS: usize = 3;

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::var_os("WSLDB_RUN_WAL_SYNC_BENCH").is_none() {
        eprintln!("set WSLDB_RUN_WAL_SYNC_BENCH=1 to run: cargo bench --bench wal_sync");
        return Ok(());
    }

    let records = environment_usize("WSLDB_WAL_SYNC_RECORDS", DEFAULT_RECORDS)?;
    let runs = environment_usize("WSLDB_WAL_SYNC_RUNS", DEFAULT_RUNS)?;
    if records == 0 || runs == 0 {
        return Err("record and run counts must be positive".into());
    }
    let directory = std::env::var_os("WSLDB_WAL_SYNC_DIRECTORY")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    if !directory.is_dir() {
        return Err(format!(
            "benchmark directory does not exist: {}",
            directory.display()
        )
        .into());
    }
    let starting_mode =
        std::env::var("WSLDB_WAL_SYNC_ORDER").unwrap_or_else(|_| "growing-first".to_owned());
    let growing_first = match starting_mode.as_str() {
        "growing-first" => true,
        "fixed-first" => false,
        _ => return Err("WAL sync order must be growing-first or fixed-first".into()),
    };

    println!(
        "pair,order,mode,records,elapsed_ns,records_per_second,p50_ns,p95_ns,p99_ns,max_ns,write_total_ns,sync_total_ns,file_bytes"
    );
    for pair in 0..runs {
        let pair_growing_first = if pair % 2 == 0 {
            growing_first
        } else {
            !growing_first
        };
        if pair_growing_first {
            run_and_report(&directory, pair, 1, Mode::Growing, records)?;
            run_and_report(&directory, pair, 2, Mode::Fixed, records)?;
        } else {
            run_and_report(&directory, pair, 1, Mode::Fixed, records)?;
            run_and_report(&directory, pair, 2, Mode::Growing, records)?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Mode {
    Growing,
    Fixed,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Growing => "growing",
            Self::Fixed => "fixed",
        }
    }
}

struct Measurement {
    elapsed: Duration,
    latencies: Vec<Duration>,
    write: Duration,
    sync: Duration,
    file_bytes: u64,
}

fn run_and_report(
    directory: &Path,
    pair: usize,
    order: usize,
    mode: Mode,
    records: usize,
) -> Result<(), Box<dyn Error>> {
    let path = directory.join(format!(
        "wsldb-wal-sync-{}-{pair}-{}.tmp",
        std::process::id(),
        mode.name()
    ));
    let measurement = run(&path, mode, records)?;
    let throughput = records as f64 / measurement.elapsed.as_secs_f64();
    let mut latencies: Vec<u128> = measurement
        .latencies
        .iter()
        .map(Duration::as_nanos)
        .collect();
    latencies.sort_unstable();
    println!(
        "{pair},{order},{},{records},{},{throughput:.3},{},{},{},{},{},{},{}",
        mode.name(),
        measurement.elapsed.as_nanos(),
        percentile(&latencies, 50),
        percentile(&latencies, 95),
        percentile(&latencies, 99),
        latencies.last().copied().unwrap_or_default(),
        measurement.write.as_nanos(),
        measurement.sync.as_nanos(),
        measurement.file_bytes,
    );
    Ok(())
}

fn run(path: &Path, mode: Mode, records: usize) -> Result<Measurement, Box<dyn Error>> {
    let mut benchmark_file = BenchmarkFile::create(path)?;
    let file = &mut benchmark_file.file;
    let file_bytes = records
        .checked_mul(RECORD_BYTES)
        .ok_or("benchmark extent is too large")?;
    match mode {
        Mode::Growing => file.sync_all()?,
        Mode::Fixed => {
            file.write_all(&vec![0_u8; file_bytes])?;
            file.sync_all()?;
            file.seek(SeekFrom::Start(0))?;
        }
    }

    let mut expected = Vec::with_capacity(file_bytes);
    let mut latencies = Vec::with_capacity(records);
    let mut write = Duration::ZERO;
    let mut sync = Duration::ZERO;
    let elapsed_started = Instant::now();
    for index in 0..records {
        let record = record(index);
        expected.extend_from_slice(&record);
        let operation_started = Instant::now();
        let write_started = Instant::now();
        file.write_all(&record)?;
        write += write_started.elapsed();
        let sync_started = Instant::now();
        file.sync_all()?;
        sync += sync_started.elapsed();
        latencies.push(operation_started.elapsed());
    }
    let elapsed = elapsed_started.elapsed();
    let actual_bytes = file.metadata()?.len();
    if actual_bytes != u64::try_from(file_bytes)? {
        return Err(format!(
            "unexpected {} file length: {actual_bytes}, expected {file_bytes}",
            mode.name()
        )
        .into());
    }
    file.seek(SeekFrom::Start(0))?;
    let mut actual = vec![0_u8; file_bytes];
    file.read_exact(&mut actual)?;
    if actual != expected {
        return Err(format!("{} file verification failed", mode.name()).into());
    }

    Ok(Measurement {
        elapsed,
        latencies,
        write,
        sync,
        file_bytes: actual_bytes,
    })
}

fn record(index: usize) -> [u8; RECORD_BYTES] {
    let mut record = [0_u8; RECORD_BYTES];
    let seed = index.to_le_bytes();
    for (position, byte) in record.iter_mut().enumerate() {
        *byte = seed[position % seed.len()] ^ (position as u8).wrapping_mul(31);
    }
    record
}

fn percentile(sorted: &[u128], percentage: usize) -> u128 {
    let rank = sorted
        .len()
        .saturating_mul(percentage)
        .div_ceil(100)
        .saturating_sub(1);
    sorted[rank]
}

fn environment_usize(name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    std::env::var(name).map_or(Ok(default), |value| Ok(value.parse()?))
}

struct BenchmarkFile {
    file: File,
    path: PathBuf,
}

impl BenchmarkFile {
    fn create(path: &Path) -> Result<Self, Box<dyn Error>> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        Ok(Self {
            file,
            path: path.to_owned(),
        })
    }
}

impl Drop for BenchmarkFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
