use std::error::Error;
use std::hint::black_box;
use std::time::{Duration, Instant};

use wsldb::{CheckpointStats, Database};

use super::{BenchmarkDatabase, Config, fill_value, key, open_database};

const DEFAULT_DURATION_SECONDS: u64 = 30 * 60;
const DEFAULT_COMPACTION_THRESHOLD_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_ZIPF_EXPONENT: f64 = 1.2;

#[derive(Clone, Copy)]
pub(super) enum Distribution {
    Uniform,
    Zipf,
}

impl Distribution {
    const fn metric_prefix(self) -> &'static str {
        match self {
            Self::Uniform => "w5",
            Self::Zipf => "w6",
        }
    }
}

pub(super) fn run(
    database_file: &BenchmarkDatabase,
    config: &Config,
    distribution: Distribution,
) -> Result<(), Box<dyn Error>> {
    let duration = environment_u64("WSLDB_BENCH_DURATION_SECONDS", DEFAULT_DURATION_SECONDS)?;
    let compaction_threshold = environment_u64(
        "WSLDB_BENCH_COMPACT_BYTES",
        DEFAULT_COMPACTION_THRESHOLD_BYTES,
    )?;
    let maintenance_seconds = environment_u64("WSLDB_BENCH_MAINTENANCE_SECONDS", 60)?;
    let operation_limit = optional_environment_u64("WSLDB_BENCH_OPERATION_LIMIT")?;
    let mut sampler = KeySampler::new(distribution, config.entries)?;
    let metric_prefix = distribution.metric_prefix();
    eprintln!("sustained_distribution={}", sampler.description());
    let mut database = open_database(database_file, config)?;
    preload_cache(&database, config.entries)?;

    let started = Instant::now();
    let deadline = started
        .checked_add(Duration::from_secs(duration))
        .ok_or("sustained workload duration is too large")?;
    let mut window_started = started;
    let mut window = Window::default();
    let mut total = Window::default();
    let mut active_elapsed = Duration::ZERO;
    let mut window_index = 0_u32;
    let mut operations = 0_u64;
    let mut random = super::SEED;
    let mut value = vec![0_u8; config.value_bytes];
    let mut checkpoint_previous = database.checkpoint_stats()?;
    let mut checkpoint_totals = CheckpointTotals::default();

    while Instant::now() < deadline && operation_limit.is_none_or(|limit| operations < limit) {
        let index = sampler.sample(&mut random);
        random = super::xorshift(random);
        let selector = random.rotate_left(23) % 100;
        let operation_key = key(index);
        let operation_started = Instant::now();
        if selector < 80 {
            let result = database.read()?.get(&operation_key)?;
            black_box(result);
            let elapsed = operation_started.elapsed();
            window.reads.push(elapsed);
            total.reads.push(elapsed);
        } else if selector < 95 {
            fill_value(index ^ operations.rotate_left(17), &mut value);
            let mut writer = database.write()?;
            writer.put(&operation_key, &value)?;
            writer.commit()?;
            let elapsed = operation_started.elapsed();
            window.writes.push(elapsed);
            total.writes.push(elapsed);
        } else {
            let mut writer = database.write()?;
            if !writer.delete(&operation_key)? {
                return Err("sustained delete selected a missing key".into());
            }
            writer.commit()?;
            fill_value(index ^ operations.rotate_left(17), &mut value);
            let mut writer = database.write()?;
            writer.put(&operation_key, &value)?;
            writer.commit()?;
            let elapsed = operation_started.elapsed();
            window.deletes.push(elapsed);
            total.deletes.push(elapsed);
        }
        operations = operations
            .checked_add(1)
            .ok_or("operation count overflow")?;

        let now = Instant::now();
        if now.duration_since(window_started) >= Duration::from_secs(1)
            || now >= deadline
            || operation_limit == Some(operations)
        {
            window_index = window_index
                .checked_add(1)
                .ok_or("sustained window count overflow")?;
            let elapsed = now.duration_since(window_started);
            active_elapsed = active_elapsed.saturating_add(elapsed);
            window.report(metric_prefix, window_index, elapsed);
            let checkpoints = database.checkpoint_stats()?;
            let checkpoint_delta = CheckpointDelta::between(checkpoint_previous, checkpoints);
            checkpoint_totals.add(checkpoint_delta);
            eprintln!(
                "sustained_window={} elapsed_seconds={:.3} total_operations={operations} checkpoint_activity={} checkpoint_starts={} checkpoint_operations={} checkpoint_failures={} checkpoint_bytes={} checkpoints_in_progress={} checkpoint_total_starts={} checkpoint_total_operations={} checkpoint_total_failures={} checkpoint_total_bytes={}",
                window_index,
                started.elapsed().as_secs_f64(),
                u8::from(
                    checkpoint_delta
                        .has_activity(checkpoint_previous.in_progress, checkpoints.in_progress,)
                ),
                checkpoint_delta.started,
                checkpoint_delta.successful,
                checkpoint_delta.failures,
                checkpoint_delta.bytes,
                checkpoints.in_progress,
                checkpoint_totals.started,
                checkpoint_totals.successful,
                checkpoint_totals.failures,
                checkpoint_totals.bytes,
            );
            checkpoint_previous = checkpoints;
            window = Window::default();

            if u64::from(window_index).is_multiple_of(maintenance_seconds) {
                let maintenance_started = Instant::now();
                let stats = database.operational_stats()?;
                let maintenance_elapsed = maintenance_started.elapsed();
                super::report_latency(
                    &format!("{metric_prefix}_maintenance"),
                    window_index,
                    &[maintenance_elapsed],
                    maintenance_elapsed,
                );
                eprintln!(
                    "sustained_maintenance={window_index} entries={} logical_hash={} commits={} commit_total_ns={} syncs={} sync_total_ns={}",
                    stats.snapshot.entries,
                    super::report::encode_hex(&stats.snapshot.logical_hash),
                    stats.commits.operations,
                    stats.commits.total_nanos,
                    stats.syncs.operations,
                    stats.syncs.total_nanos,
                );
                let after_maintenance = database.checkpoint_stats()?;
                let maintenance_delta =
                    CheckpointDelta::between(checkpoint_previous, after_maintenance);
                checkpoint_totals.add(maintenance_delta);
                if maintenance_delta.has_activity(
                    checkpoint_previous.in_progress,
                    after_maintenance.in_progress,
                ) {
                    eprintln!(
                        "sustained_maintenance_checkpoint={} checkpoint_starts={} checkpoint_operations={} checkpoint_failures={} checkpoint_bytes={} checkpoints_in_progress={}",
                        window_index,
                        maintenance_delta.started,
                        maintenance_delta.successful,
                        maintenance_delta.failures,
                        maintenance_delta.bytes,
                        after_maintenance.in_progress,
                    );
                }
                checkpoint_previous = after_maintenance;
                if stats.retired_bytes >= compaction_threshold {
                    let checkpoint_drain_started = Instant::now();
                    database.flush()?;
                    let checkpoint_drain_elapsed = checkpoint_drain_started.elapsed();
                    super::report_latency(
                        &format!("{metric_prefix}_pre_compaction_drain"),
                        window_index,
                        &[checkpoint_drain_elapsed],
                        checkpoint_drain_elapsed,
                    );
                    let after_checkpoint_drain = database.checkpoint_stats()?;
                    let checkpoint_drain_delta =
                        CheckpointDelta::between(checkpoint_previous, after_checkpoint_drain);
                    checkpoint_totals.add(checkpoint_drain_delta);
                    eprintln!(
                        "sustained_pre_compaction_drain={} elapsed_ns={} checkpoint_activity={} checkpoint_starts={} checkpoint_operations={} checkpoint_failures={} checkpoint_bytes={} checkpoints_in_progress={}",
                        window_index,
                        checkpoint_drain_elapsed.as_nanos(),
                        u8::from(checkpoint_drain_delta.has_activity(
                            checkpoint_previous.in_progress,
                            after_checkpoint_drain.in_progress,
                        )),
                        checkpoint_drain_delta.started,
                        checkpoint_drain_delta.successful,
                        checkpoint_drain_delta.failures,
                        checkpoint_drain_delta.bytes,
                        after_checkpoint_drain.in_progress,
                    );
                    if after_checkpoint_drain.in_progress != 0 {
                        return Err("pre-compaction drain left checkpoint work active".into());
                    }
                    drop(database);
                    let compact_started = Instant::now();
                    let report = Database::compact_file(database_file.path())?;
                    let elapsed = compact_started.elapsed();
                    super::report_latency(
                        &format!("{metric_prefix}_compaction"),
                        window_index,
                        &[elapsed],
                        elapsed,
                    );
                    eprintln!(
                        "sustained_compaction={} source_bytes={} compacted_bytes={}",
                        window_index, report.source_bytes, report.compacted_bytes,
                    );
                    database = open_database(database_file, config)?;
                    checkpoint_previous = database.checkpoint_stats()?;
                }
            }
            // Keep reporting and maintenance outside the next active window.
            window_started = Instant::now();
        }
    }

    total.report(&format!("{metric_prefix}_total"), 0, active_elapsed);
    let drain_started = Instant::now();
    database.flush()?;
    let drain_elapsed = drain_started.elapsed();
    super::report_latency(
        &format!("{metric_prefix}_drain"),
        0,
        &[drain_elapsed],
        drain_elapsed,
    );
    eprintln!("sustained_drain elapsed_ns={}", drain_elapsed.as_nanos());
    let after_drain = database.checkpoint_stats()?;
    let drain_delta = CheckpointDelta::between(checkpoint_previous, after_drain);
    checkpoint_totals.add(drain_delta);
    eprintln!(
        "sustained_drain_checkpoint checkpoint_activity={} checkpoint_starts={} checkpoint_operations={} checkpoint_failures={} checkpoint_bytes={} checkpoints_in_progress={}",
        u8::from(
            drain_delta.has_activity(checkpoint_previous.in_progress, after_drain.in_progress,)
        ),
        drain_delta.started,
        drain_delta.successful,
        drain_delta.failures,
        drain_delta.bytes,
        after_drain.in_progress,
    );
    if after_drain.in_progress != 0 {
        return Err("final drain left checkpoint work active".into());
    }
    let stats = database.operational_stats()?;
    let verification = stats.snapshot;
    if verification.entries != config.entries {
        return Err("sustained workload did not preserve its fixed-size dataset".into());
    }
    eprintln!(
        "sustained_complete operations={operations} entries={} logical_hash={} checkpoint_total_starts={} checkpoint_total_operations={} checkpoint_total_failures={} checkpoint_total_bytes={}",
        verification.entries,
        super::report::encode_hex(&verification.logical_hash),
        checkpoint_totals.started,
        checkpoint_totals.successful,
        checkpoint_totals.failures,
        checkpoint_totals.bytes,
    );
    super::report::database(&database, database_file, config)?;
    super::report::operational(&stats, database_file)
}

#[derive(Clone, Copy, Default)]
struct CheckpointDelta {
    started: u64,
    successful: u64,
    failures: u64,
    bytes: u64,
}

impl CheckpointDelta {
    fn between(previous: CheckpointStats, current: CheckpointStats) -> Self {
        Self {
            started: current.started.saturating_sub(previous.started),
            successful: current
                .successful
                .operations
                .saturating_sub(previous.successful.operations),
            failures: current.failures.saturating_sub(previous.failures),
            bytes: current
                .database_bytes
                .saturating_sub(previous.database_bytes),
        }
    }

    const fn has_activity(self, active_before: u64, active_after: u64) -> bool {
        active_before != 0
            || active_after != 0
            || self.started != 0
            || self.successful != 0
            || self.failures != 0
    }
}

#[derive(Default)]
struct CheckpointTotals {
    started: u64,
    successful: u64,
    failures: u64,
    bytes: u64,
}

impl CheckpointTotals {
    fn add(&mut self, delta: CheckpointDelta) {
        self.started = self.started.saturating_add(delta.started);
        self.successful = self.successful.saturating_add(delta.successful);
        self.failures = self.failures.saturating_add(delta.failures);
        self.bytes = self.bytes.saturating_add(delta.bytes);
    }
}

fn preload_cache(database: &Database, expected_entries: u64) -> Result<(), Box<dyn Error>> {
    const BATCH_ENTRIES: u64 = 1_024;

    let mut first = 0_u64;
    let mut observed = 0_u64;
    while first < expected_entries {
        let end = first.saturating_add(BATCH_ENTRIES).min(expected_entries);
        let start_key = key(first);
        let end_key = (end < expected_entries).then(|| key(end));
        let limit = usize::try_from(end - first)?;
        let entries = database.read()?.range(
            &start_key,
            end_key.as_ref().map(|end_key| end_key.as_slice()),
            limit,
        )?;
        for entry in entries {
            black_box(entry);
            observed = observed.checked_add(1).ok_or("entry count overflow")?;
        }
        first = end;
    }
    if observed != expected_entries {
        return Err("database entry count does not match sustained workload configuration".into());
    }
    eprintln!("sustained_preload entries={observed}");
    Ok(())
}

enum KeySampler {
    Uniform { entries: u64 },
    Zipf(Zipf),
}

impl KeySampler {
    fn new(distribution: Distribution, entries: u64) -> Result<Self, Box<dyn Error>> {
        match distribution {
            Distribution::Uniform => Ok(Self::Uniform { entries }),
            Distribution::Zipf => {
                let exponent = environment_f64("WSLDB_BENCH_ZIPF_EXPONENT", DEFAULT_ZIPF_EXPONENT)?;
                Ok(Self::Zipf(Zipf::new(entries, exponent)?))
            }
        }
    }

    fn sample(&mut self, random: &mut u64) -> u64 {
        match self {
            Self::Uniform { entries } => {
                *random = super::xorshift(*random);
                *random % *entries
            }
            Self::Zipf(zipf) => zipf.sample(random),
        }
    }

    fn description(&self) -> String {
        match self {
            Self::Uniform { .. } => "uniform".to_owned(),
            Self::Zipf(zipf) => format!("zipf exponent={}", zipf.exponent),
        }
    }
}

struct Zipf {
    entries: u64,
    exponent: f64,
    h_integral_x1: f64,
    h_integral_entries: f64,
    acceptance: f64,
}

impl Zipf {
    fn new(entries: u64, exponent: f64) -> Result<Self, Box<dyn Error>> {
        if entries == 0 || !exponent.is_finite() || exponent <= 0.0 {
            return Err("Zipf entries and exponent must be positive".into());
        }
        let h_integral_x1 = h_integral(1.5, exponent) - 1.0;
        let h_integral_entries = h_integral(entries as f64 + 0.5, exponent);
        let acceptance =
            2.0 - h_integral_inverse(h_integral(2.5, exponent) - h(2.0, exponent), exponent);
        Ok(Self {
            entries,
            exponent,
            h_integral_x1,
            h_integral_entries,
            acceptance,
        })
    }

    fn sample(&self, random: &mut u64) -> u64 {
        loop {
            *random = super::xorshift(*random);
            let uniform = (*random >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0);
            let u =
                self.h_integral_entries + uniform * (self.h_integral_x1 - self.h_integral_entries);
            let candidate = h_integral_inverse(u, self.exponent);
            let rank = (candidate + 0.5).floor().clamp(1.0, self.entries as f64) as u64;
            if rank as f64 - candidate <= self.acceptance
                || u >= h_integral(rank as f64 + 0.5, self.exponent) - h(rank as f64, self.exponent)
            {
                return rank - 1;
            }
        }
    }
}

fn h(value: f64, exponent: f64) -> f64 {
    (-exponent * value.ln()).exp()
}

fn h_integral(value: f64, exponent: f64) -> f64 {
    let log = value.ln();
    log * expm1_over_x((1.0 - exponent) * log)
}

fn h_integral_inverse(value: f64, exponent: f64) -> f64 {
    (value * log1p_over_x((1.0 - exponent) * value)).exp()
}

fn expm1_over_x(value: f64) -> f64 {
    if value.abs() > 1.0e-8 {
        value.exp_m1() / value
    } else {
        1.0 + value * (0.5 + value * (1.0 / 6.0 + value / 24.0))
    }
}

fn log1p_over_x(value: f64) -> f64 {
    if value.abs() > 1.0e-8 {
        value.ln_1p() / value
    } else {
        1.0 - value * (0.5 - value * (1.0 / 3.0 - value / 4.0))
    }
}

fn environment_u64(name: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    let value = match std::env::var(name) {
        Ok(value) => value.parse::<u64>()?,
        Err(std::env::VarError::NotPresent) => default,
        Err(error) => return Err(error.into()),
    };
    if value == 0 {
        return Err(format!("{name} must be non-zero").into());
    }
    Ok(value)
}

fn optional_environment_u64(name: &str) -> Result<Option<u64>, Box<dyn Error>> {
    match std::env::var(name) {
        Ok(value) => {
            let value = value.parse::<u64>()?;
            if value == 0 {
                return Err(format!("{name} must be non-zero when set").into());
            }
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn environment_f64(name: &str, default: f64) -> Result<f64, Box<dyn Error>> {
    let value = match std::env::var(name) {
        Ok(value) => value.parse::<f64>()?,
        Err(std::env::VarError::NotPresent) => default,
        Err(error) => return Err(error.into()),
    };
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{name} must be a positive finite number").into());
    }
    Ok(value)
}

#[derive(Default)]
struct Window {
    reads: Vec<Duration>,
    writes: Vec<Duration>,
    deletes: Vec<Duration>,
}

impl Window {
    fn report(&self, prefix: &str, index: u32, elapsed: Duration) {
        report_samples(&format!("{prefix}_read"), index, &self.reads, elapsed);
        report_samples(&format!("{prefix}_overwrite"), index, &self.writes, elapsed);
        report_samples(&format!("{prefix}_delete"), index, &self.deletes, elapsed);
        let operations = self.reads.len() + self.writes.len() + self.deletes.len();
        super::report_throughput(
            &format!("{prefix}_all"),
            index,
            operations.try_into().unwrap_or(u64::MAX),
            elapsed,
        );
    }
}

fn report_samples(metric: &str, index: u32, samples: &[Duration], elapsed: Duration) {
    if !samples.is_empty() {
        super::report_latency(metric, index, samples, elapsed);
    }
}
