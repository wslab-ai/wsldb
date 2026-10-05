#[path = "wsldb/measurement.rs"]
mod measurement;
#[path = "wsldb/report.rs"]
mod report;
#[path = "wsldb/sustained.rs"]
mod sustained;

use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use wsldb::{Database, DatabaseOptions, ReadTransaction};

use measurement::MeasurementPlan;

const DEFAULT_ENTRIES: u64 = 100_000;
const DEFAULT_OPERATIONS: u64 = 100_000;
const DEFAULT_BATCH_SIZE: u64 = 1_000;
const DEFAULT_RANGE_SIZE: usize = 100;
const DEFAULT_VALUE_BYTES: usize = 100;
const READ_THREAD_COUNTS: [usize; 4] = [1, 4, 16, 64];
const MAX_RANGE_WARMUP_OPERATIONS: u64 = 1_000_000;
const ESTIMATED_ENTRIES_PER_LEAF: u64 = 32;
const SEED: u64 = 0x4D59_5DF4_D0F3_3173;

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::var_os("WSLDB_RUN_BENCHMARK").is_none() {
        eprintln!(
            "set WSLDB_RUN_BENCHMARK=1 to run: cargo bench --bench wsldb -- [entries] [operations] [batch] [cache_pages]"
        );
        return Ok(());
    }
    let config = Config::from_args()?;
    let mode = Mode::from_environment()?;
    let database_file = BenchmarkDatabase::new()?;
    println!("metric,run,operations,elapsed_ns,ops_per_second,p50_ns,p95_ns,p99_ns,p999_ns");

    if mode.loads() {
        if database_file.path().exists() {
            return Err("load mode requires a non-existing database path".into());
        }
        let database = open_database(&database_file, &config)?;
        let load_started = Instant::now();
        let order = mode.load_order();
        load(&database, &config, order)?;
        if database.stats()?.entries != config.entries {
            return Err("load did not create the configured number of entries".into());
        }
        report_throughput(order.metric(), 0, config.entries, load_started.elapsed());
        let drain_elapsed = if std::env::var_os("WSLDB_BENCH_DRAIN").is_some() {
            let started = Instant::now();
            database.flush()?;
            Some(started.elapsed())
        } else {
            None
        };
        report::load_database(&database, &database_file, &config)?;
        drop(database);
        if matches!(mode, Mode::Load | Mode::RandomLoad) {
            if let Some(drain_elapsed) = drain_elapsed {
                eprintln!("drain_ns={}", drain_elapsed.as_nanos());
            }
            return Ok(());
        }
    }

    if let Some(distribution) = mode.sustained_distribution() {
        return sustained::run(&database_file, &config, distribution);
    }
    let measurement = MeasurementPlan::from_environment()?;

    let open_started = Instant::now();
    let database = open_database(&database_file, &config)?;
    if database.stats()?.entries != config.entries {
        return Err("database entry count does not match benchmark configuration".into());
    }
    if mode.opens() {
        let first = database.read()?.get(&key(0))?;
        black_box(first);
        let open_elapsed = open_started.elapsed();
        report_latency(
            "w7_open_to_first_read",
            measurement.first_run(),
            &[open_elapsed],
            open_elapsed,
        );
    }

    if mode.reads() {
        for threads in read_thread_counts()? {
            if threads as u64 > config.operations {
                continue;
            }
            if measurement.warmup() {
                let _ = point_reads_parallel(&database, config.operations, threads, SEED, false)?;
            }
            for run in measurement.runs() {
                let (mut samples, elapsed) = point_reads_parallel(
                    &database,
                    config.operations,
                    threads,
                    SEED + u64::from(run),
                    true,
                )?;
                report_parallel_latency(
                    &format!("w3_random_point_reads_t{threads}"),
                    run,
                    &mut samples,
                    elapsed,
                );
            }
        }
    }

    if mode.ranges() {
        let reader = database.read()?;
        let range_operations = config.operations.min(10_000);
        let range_size = u64::try_from(config.range_size)?;
        let cache_warmup_operations = u64::try_from(config.cache_pages)?
            .saturating_mul(5)
            .saturating_mul(ESTIMATED_ENTRIES_PER_LEAF)
            .div_ceil(range_size)
            .min(MAX_RANGE_WARMUP_OPERATIONS);
        if measurement.warmup() {
            let warmup_operations = range_operations.max(cache_warmup_operations);
            let _ = range_scans(
                &reader,
                config.entries,
                warmup_operations,
                config.range_size,
                SEED,
                false,
            )?;
        }
        for run in measurement.runs() {
            let started = Instant::now();
            let samples = range_scans(
                &reader,
                config.entries,
                range_operations,
                config.range_size,
                SEED + u64::from(run),
                true,
            )?;
            report_latency(
                &format!("w4_range_{}", config.range_size),
                run,
                &samples,
                started.elapsed(),
            );
        }
    }

    report::database(&database, &database_file, &config)
}

fn open_database(
    database_file: &BenchmarkDatabase,
    config: &Config,
) -> Result<Database, Box<dyn Error>> {
    Database::open_with_options(
        database_file.path(),
        DatabaseOptions::new()
            .page_cache_capacity(config.cache_pages)
            .write_buffer_bytes(config.write_buffer_bytes),
    )
    .map_err(Into::into)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Mode {
    All,
    Load,
    RandomLoad,
    Reads,
    Ranges,
    Open,
    Sustained,
    Skewed,
}

impl Mode {
    fn from_environment() -> Result<Self, Box<dyn Error>> {
        match std::env::var("WSLDB_BENCH_WORKLOAD") {
            Err(std::env::VarError::NotPresent) => Ok(Self::All),
            Ok(value) if value == "all" => Ok(Self::All),
            Ok(value) if value == "load" => Ok(Self::Load),
            Ok(value) if value == "random-load" => Ok(Self::RandomLoad),
            Ok(value) if value == "reads" => Ok(Self::Reads),
            Ok(value) if value == "ranges" => Ok(Self::Ranges),
            Ok(value) if value == "open" => Ok(Self::Open),
            Ok(value) if value == "sustained" => Ok(Self::Sustained),
            Ok(value) if value == "skewed" => Ok(Self::Skewed),
            Ok(_) => Err(
                "WSLDB_BENCH_WORKLOAD must be all, load, random-load, reads, ranges, open, sustained, or skewed"
                    .into(),
            ),
            Err(error) => Err(error.into()),
        }
    }

    const fn loads(self) -> bool {
        matches!(self, Self::All | Self::Load | Self::RandomLoad)
    }

    const fn reads(self) -> bool {
        matches!(self, Self::All | Self::Reads)
    }

    const fn ranges(self) -> bool {
        matches!(self, Self::All | Self::Ranges)
    }

    const fn opens(self) -> bool {
        matches!(self, Self::All | Self::Open)
    }

    const fn load_order(self) -> LoadOrder {
        if matches!(self, Self::RandomLoad) {
            LoadOrder::Random
        } else {
            LoadOrder::Sequential
        }
    }

    const fn sustained_distribution(self) -> Option<sustained::Distribution> {
        match self {
            Self::Sustained => Some(sustained::Distribution::Uniform),
            Self::Skewed => Some(sustained::Distribution::Zipf),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum LoadOrder {
    Sequential,
    Random,
}

impl LoadOrder {
    const fn metric(self) -> &'static str {
        match self {
            Self::Sequential => "w1_sequential_load",
            Self::Random => "w2_random_load",
        }
    }
}

struct Config {
    entries: u64,
    operations: u64,
    batch_size: u64,
    cache_pages: usize,
    range_size: usize,
    value_bytes: usize,
    write_buffer_bytes: u64,
}

impl Config {
    fn from_args() -> Result<Self, Box<dyn Error>> {
        let mut arguments = std::env::args().skip(1).filter(|value| value != "--bench");
        let entries = parse_or(arguments.next(), DEFAULT_ENTRIES)?;
        let operations = parse_or(arguments.next(), DEFAULT_OPERATIONS)?;
        let batch_size = parse_or(arguments.next(), DEFAULT_BATCH_SIZE)?;
        let cache_pages = parse_or(arguments.next(), 4096_usize)?;
        let range_size = std::env::var("WSLDB_BENCH_RANGE")
            .map_or(Ok(DEFAULT_RANGE_SIZE), |value| {
                parse_or(Some(value), 0_usize)
            })?;
        let value_bytes = std::env::var("WSLDB_BENCH_VALUE_BYTES")
            .map_or(Ok(DEFAULT_VALUE_BYTES), |value| {
                parse_or(Some(value), 0_usize)
            })?;
        let write_buffer_bytes = std::env::var("WSLDB_BENCH_WRITE_BUFFER_BYTES")
            .map_or(Ok(0_u64), |value| parse_or(Some(value), 0_u64))?;
        if entries == 0 || operations == 0 || batch_size == 0 {
            return Err("entries, operations, and batch size must be non-zero".into());
        }
        if range_size == 0 {
            return Err("WSLDB_BENCH_RANGE must be non-zero".into());
        }
        if value_bytes == 0 {
            return Err("WSLDB_BENCH_VALUE_BYTES must be non-zero".into());
        }
        if arguments.next().is_some() {
            return Err(
                "usage: cargo bench --bench wsldb -- [entries] [operations] [batch] [cache_pages]"
                    .into(),
            );
        }
        Ok(Self {
            entries,
            operations,
            batch_size,
            cache_pages,
            range_size,
            value_bytes,
            write_buffer_bytes,
        })
    }
}

fn read_thread_counts() -> Result<Vec<usize>, Box<dyn Error>> {
    let Some(value) = std::env::var_os("WSLDB_BENCH_THREADS") else {
        return Ok(READ_THREAD_COUNTS.to_vec());
    };
    let threads = value.to_string_lossy().parse::<usize>()?;
    if !READ_THREAD_COUNTS.contains(&threads) {
        return Err("WSLDB_BENCH_THREADS must be 1, 4, 16, or 64".into());
    }
    Ok(vec![threads])
}

fn parse_or<T>(argument: Option<String>, default: T) -> Result<T, Box<dyn Error>>
where
    T: std::str::FromStr,
    T::Err: Error + 'static,
{
    argument.map_or(Ok(default), |value| {
        value
            .parse::<T>()
            .map_err(|error| Box::new(error) as Box<dyn Error>)
    })
}

fn load(database: &Database, config: &Config, order: LoadOrder) -> Result<(), Box<dyn Error>> {
    let permutation = Permutation::new(config.entries);
    let mut value_bytes = vec![0_u8; config.value_bytes];
    let mut first = 0;
    while first < config.entries {
        let end = first.saturating_add(config.batch_size).min(config.entries);
        let mut writer = database.write()?;
        for position in first..end {
            let index = match order {
                LoadOrder::Sequential => position,
                LoadOrder::Random => permutation.apply(position),
            };
            fill_value(index, &mut value_bytes);
            writer.put(&key(index), &value_bytes)?;
        }
        writer.commit()?;
        first = end;
    }
    Ok(())
}

struct Permutation {
    modulus: u64,
    multiplier: u64,
    offset: u64,
}

impl Permutation {
    fn new(modulus: u64) -> Self {
        let mut multiplier = (SEED | 1) % modulus;
        while greatest_common_divisor(multiplier, modulus) != 1 {
            multiplier = ((u128::from(multiplier) + 2) % u128::from(modulus)) as u64;
        }
        Self {
            modulus,
            multiplier,
            offset: SEED.rotate_left(29) % modulus,
        }
    }

    fn apply(&self, position: u64) -> u64 {
        let product = u128::from(position) * u128::from(self.multiplier);
        ((product + u128::from(self.offset)) % u128::from(self.modulus)) as u64
    }
}

fn greatest_common_divisor(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn point_reads(
    reader: &ReadTransaction,
    operations: u64,
    mut random: u64,
    record: bool,
) -> Result<Vec<u64>, Box<dyn Error>> {
    let capacity = usize::try_from(operations)?;
    let mut samples = Vec::new();
    if record {
        samples.try_reserve_exact(capacity)?;
    }
    let entries = reader.len();
    for operation in 0..operations {
        random = xorshift(random);
        let present = operation % 2 == 0;
        let index = random % entries + if present { 0 } else { entries };
        let started = Instant::now();
        let value = reader.get(&key(index))?;
        let elapsed = started.elapsed();
        if value.is_some() != present {
            return Err("point-read result did not match the generated workload".into());
        }
        black_box(value);
        if record {
            samples.push(u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX));
        }
    }
    Ok(samples)
}

fn point_reads_parallel(
    database: &Database,
    operations: u64,
    threads: usize,
    seed: u64,
    record: bool,
) -> Result<(ParallelLatency, Duration), Box<dyn Error>> {
    let barrier = Arc::new(Barrier::new(threads + 1));
    thread::scope(
        |scope| -> Result<(ParallelLatency, Duration), Box<dyn Error>> {
            let mut workers = Vec::with_capacity(threads);
            for worker in 0..threads {
                let database = database.clone();
                let barrier = Arc::clone(&barrier);
                let worker_operations = operations / threads as u64
                    + u64::from(worker < (operations % threads as u64) as usize);
                workers.push(scope.spawn(move || -> Result<Vec<u64>, String> {
                    let reader = database.read().map_err(|error| error.to_string())?;
                    barrier.wait();
                    point_reads(
                        &reader,
                        worker_operations,
                        seed ^ (worker as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                        record,
                    )
                    .map_err(|error| error.to_string())
                }));
            }
            barrier.wait();
            let started = Instant::now();
            let mut samples = ParallelLatency::new(workers.len())?;
            for worker in workers {
                samples.push(worker.join().map_err(|_| "point-read worker panicked")??);
            }
            Ok((samples, started.elapsed()))
        },
    )
}

struct ParallelLatency {
    shards: Vec<Vec<u64>>,
    samples: usize,
}

impl ParallelLatency {
    fn new(capacity: usize) -> Result<Self, Box<dyn Error>> {
        let mut shards = Vec::new();
        shards.try_reserve_exact(capacity)?;
        Ok(Self { shards, samples: 0 })
    }

    fn push(&mut self, shard: Vec<u64>) {
        self.samples = self.samples.saturating_add(shard.len());
        self.shards.push(shard);
    }

    fn sort(&mut self) {
        for shard in &mut self.shards {
            shard.sort_unstable();
        }
    }

    fn percentile(&self, permille: usize) -> u64 {
        if self.samples == 0 {
            return 0;
        }
        let rank = self
            .samples
            .saturating_mul(permille)
            .div_ceil(1000)
            .saturating_sub(1);
        let Some(mut low) = self
            .shards
            .iter()
            .filter_map(|shard| shard.first())
            .min()
            .copied()
        else {
            return 0;
        };
        let Some(mut high) = self
            .shards
            .iter()
            .filter_map(|shard| shard.last())
            .max()
            .copied()
        else {
            return 0;
        };
        while low < high {
            let middle = low + (high - low) / 2;
            let count = self.shards.iter().fold(0_usize, |total, shard| {
                total.saturating_add(shard.partition_point(|sample| *sample <= middle))
            });
            if count > rank {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        low
    }
}

fn range_scans(
    reader: &ReadTransaction,
    entries: u64,
    operations: u64,
    limit: usize,
    mut random: u64,
    record: bool,
) -> Result<Vec<Duration>, Box<dyn Error>> {
    let capacity = usize::try_from(operations)?;
    let mut samples = Vec::new();
    if record {
        samples.try_reserve_exact(capacity)?;
    }
    for _ in 0..operations {
        random = xorshift(random);
        let start = random % entries;
        let started = Instant::now();
        let count = reader.range(&key(start), None, limit)?.count();
        let elapsed = started.elapsed();
        black_box(count);
        if record {
            samples.push(elapsed);
        }
    }
    Ok(samples)
}

fn key(index: u64) -> [u8; 16] {
    // Match RocksDB db_bench's GenerateKeyFromInt representation exactly:
    // an eight-byte big-endian integer followed by ASCII zero padding.
    let mut bytes = [b'0'; 16];
    bytes[..8].copy_from_slice(&index.to_be_bytes());
    bytes
}

fn fill_value(index: u64, bytes: &mut [u8]) {
    let mut random = index ^ SEED;
    for byte in bytes {
        random = xorshift(random);
        *byte = random as u8;
    }
}

fn xorshift(mut value: u64) -> u64 {
    value ^= value << 13;
    value ^= value >> 7;
    value ^ (value << 17)
}

fn report_throughput(metric: &str, run: u32, operations: u64, elapsed: Duration) {
    let rate = operations as f64 / elapsed.as_secs_f64();
    println!(
        "{metric},{run},{operations},{},{rate:.3},,,,,",
        elapsed.as_nanos()
    );
}

fn report_latency(metric: &str, run: u32, samples: &[Duration], elapsed: Duration) {
    let mut nanos: Vec<u128> = samples.iter().map(Duration::as_nanos).collect();
    nanos.sort_unstable();
    let rate = nanos.len() as f64 / elapsed.as_secs_f64();
    println!(
        "{metric},{run},{},{},{rate:.3},{},{},{},{}",
        nanos.len(),
        elapsed.as_nanos(),
        percentile(&nanos, 500),
        percentile(&nanos, 950),
        percentile(&nanos, 990),
        percentile(&nanos, 999),
    );
}

fn report_parallel_latency(
    metric: &str,
    run: u32,
    samples: &mut ParallelLatency,
    elapsed: Duration,
) {
    samples.sort();
    let rate = samples.samples as f64 / elapsed.as_secs_f64();
    println!(
        "{metric},{run},{},{},{rate:.3},{},{},{},{}",
        samples.samples,
        elapsed.as_nanos(),
        samples.percentile(500),
        samples.percentile(950),
        samples.percentile(990),
        samples.percentile(999),
    );
}

fn percentile(sorted: &[u128], permille: usize) -> u128 {
    let index = sorted
        .len()
        .saturating_mul(permille)
        .div_ceil(1000)
        .saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

struct BenchmarkDatabase {
    path: PathBuf,
    remove_on_drop: bool,
}

impl BenchmarkDatabase {
    fn new() -> Result<Self, Box<dyn Error>> {
        if let Some(path) = std::env::var_os("WSLDB_BENCH_DATABASE") {
            return Ok(Self {
                path: PathBuf::from(path),
                remove_on_drop: false,
            });
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let name = format!(
            "wsldb-bench-{}-{timestamp}-{}.wsldb",
            std::process::id(),
            SEED
        );
        Ok(Self {
            path: std::env::temp_dir().join(name),
            remove_on_drop: true,
        })
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for BenchmarkDatabase {
    fn drop(&mut self) {
        if !self.remove_on_drop {
            return;
        }
        remove_benchmark_file(&self.path);
        for suffix in [".lock", ".wal"] {
            let mut sidecar = self.path.as_os_str().to_owned();
            sidecar.push(suffix);
            remove_benchmark_file(std::path::Path::new(&sidecar));
        }
    }
}

fn remove_benchmark_file(path: &std::path::Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("failed to remove {}: {error}", path.display());
    }
}
