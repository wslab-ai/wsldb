use std::error::Error;
use std::ops::RangeInclusive;

const DEFAULT_MEASURED_RUNS: u32 = 5;
const DEFAULT_RUN_START: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MeasurementPlan {
    run_start: u32,
    measured_runs: u32,
    warmup: bool,
}

impl MeasurementPlan {
    pub(crate) fn from_environment() -> Result<Self, Box<dyn Error>> {
        Self::from_values(
            std::env::var("WSLDB_BENCH_RUN_START").ok().as_deref(),
            std::env::var("WSLDB_BENCH_MEASURED_RUNS").ok().as_deref(),
            std::env::var("WSLDB_BENCH_WARMUP").ok().as_deref(),
        )
    }

    pub(crate) fn from_values(
        run_start: Option<&str>,
        measured_runs: Option<&str>,
        warmup: Option<&str>,
    ) -> Result<Self, Box<dyn Error>> {
        let run_start = parse_nonnegative(run_start, DEFAULT_RUN_START)?;
        let measured_runs = parse_positive(
            measured_runs,
            DEFAULT_MEASURED_RUNS,
            "WSLDB_BENCH_MEASURED_RUNS",
        )?;
        let warmup = match warmup {
            None | Some("1") => true,
            Some("0") => false,
            Some(_) => return Err("WSLDB_BENCH_WARMUP must be 0 or 1".into()),
        };
        run_start
            .checked_add(measured_runs - 1)
            .ok_or("benchmark run IDs overflow u32")?;
        Ok(Self {
            run_start,
            measured_runs,
            warmup,
        })
    }

    pub(crate) const fn warmup(self) -> bool {
        self.warmup
    }

    pub(crate) fn runs(self) -> RangeInclusive<u32> {
        self.run_start..=self.run_start + self.measured_runs - 1
    }

    pub(crate) const fn first_run(self) -> u32 {
        self.run_start
    }
}

fn parse_positive(value: Option<&str>, default: u32, name: &str) -> Result<u32, Box<dyn Error>> {
    let parsed = value.map_or(Ok(default), str::parse::<u32>)?;
    if parsed == 0 {
        return Err(format!("{name} must be positive").into());
    }
    Ok(parsed)
}

fn parse_nonnegative(value: Option<&str>, default: u32) -> Result<u32, Box<dyn Error>> {
    value
        .map_or(Ok(default), str::parse::<u32>)
        .map_err(Into::into)
}
