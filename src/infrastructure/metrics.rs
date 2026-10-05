use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::{CheckpointStats, LatencyStats, WriteStats};

const HISTOGRAM_SUB_BUCKET_BITS: u32 = 4;
const HISTOGRAM_SUB_BUCKETS: usize = 1 << HISTOGRAM_SUB_BUCKET_BITS;
const HISTOGRAM_EXACT_BUCKETS: usize = HISTOGRAM_SUB_BUCKETS;
const HISTOGRAM_EXPONENTS: usize = u64::BITS as usize - HISTOGRAM_SUB_BUCKET_BITS as usize;
const HISTOGRAM_BUCKETS: usize =
    HISTOGRAM_EXACT_BUCKETS + HISTOGRAM_EXPONENTS * HISTOGRAM_SUB_BUCKETS;

#[derive(Default)]
pub(crate) struct RuntimeMetrics {
    commits: TimingMetrics,
    syncs: TimingMetrics,
    wal_preflights: TimingMetrics,
    wal_encodes: TimingMetrics,
    wal_writes: TimingMetrics,
    wal_syncs: TimingMetrics,
    wal_publications: TimingMetrics,
    checkpoints: TimingMetrics,
    checkpoint_starts: AtomicU64,
    checkpoints_in_progress: AtomicU64,
    checkpoint_bytes: AtomicU64,
    logical_mutation_bytes: AtomicU64,
    data_page_bytes: AtomicU64,
    leaf_page_bytes: AtomicU64,
    branch_page_bytes: AtomicU64,
    overflow_page_bytes: AtomicU64,
    write_buffer_bytes: AtomicU64,
    metadata_bytes: AtomicU64,
    corruption_detections: AtomicU64,
    checkpoint_failures: AtomicU64,
}

impl RuntimeMetrics {
    pub(crate) fn checkpoint_activity(self: &Arc<Self>) -> CheckpointActivity {
        add_saturating(&self.checkpoint_starts, 1);
        add_saturating(&self.checkpoints_in_progress, 1);
        CheckpointActivity {
            metrics: Arc::clone(self),
        }
    }

    pub(crate) fn record_commit(&self, commit: Duration, sync: Duration, writes: WriteStats) {
        self.commits.record(commit);
        self.record_sync_and_writes(sync, writes);
    }

    pub(crate) fn record_maintenance(&self, sync: Duration, writes: WriteStats) {
        self.record_sync_and_writes(sync, writes);
    }

    pub(crate) fn record_checkpoint_completion(&self, elapsed: Duration, database_bytes: u64) {
        self.checkpoints.record(elapsed);
        add_saturating(&self.checkpoint_bytes, database_bytes);
    }

    pub(crate) fn record_wal_append(&self, encode: Duration, write: Duration, sync: Duration) {
        self.wal_encodes.record(encode);
        self.wal_writes.record(write);
        self.wal_syncs.record(sync);
    }

    pub(crate) fn record_wal_commit_edges(&self, preflight: Duration, publication: Duration) {
        self.wal_preflights.record(preflight);
        self.wal_publications.record(publication);
    }

    fn record_sync_and_writes(&self, sync: Duration, writes: WriteStats) {
        self.syncs.record(sync);
        add_saturating(&self.logical_mutation_bytes, writes.logical_mutation_bytes);
        add_saturating(&self.data_page_bytes, writes.data_page_bytes);
        add_saturating(&self.leaf_page_bytes, writes.leaf_page_bytes);
        add_saturating(&self.branch_page_bytes, writes.branch_page_bytes);
        add_saturating(&self.overflow_page_bytes, writes.overflow_page_bytes);
        add_saturating(&self.write_buffer_bytes, writes.write_buffer_bytes);
        add_saturating(&self.metadata_bytes, writes.metadata_bytes);
    }

    pub(crate) fn record_corruption(&self) {
        self.corruption_detections.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_checkpoint_failure(&self) {
        self.checkpoint_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn checkpoint_stats(&self) -> CheckpointStats {
        CheckpointStats {
            started: self.checkpoint_starts.load(Ordering::Relaxed),
            in_progress: self.checkpoints_in_progress.load(Ordering::Relaxed),
            successful: self.checkpoints.snapshot(),
            failures: self.checkpoint_failures.load(Ordering::Relaxed),
            database_bytes: self.checkpoint_bytes.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            commits: self.commits.snapshot(),
            syncs: self.syncs.snapshot(),
            wal_preflights: self.wal_preflights.snapshot(),
            wal_encodes: self.wal_encodes.snapshot(),
            wal_writes: self.wal_writes.snapshot(),
            wal_syncs: self.wal_syncs.snapshot(),
            wal_publications: self.wal_publications.snapshot(),
            checkpoints: self.checkpoints.snapshot(),
            checkpoint_starts: self.checkpoint_starts.load(Ordering::Relaxed),
            checkpoints_in_progress: self.checkpoints_in_progress.load(Ordering::Relaxed),
            checkpoint_bytes: self.checkpoint_bytes.load(Ordering::Relaxed),
            writes: WriteStats {
                logical_mutation_bytes: self.logical_mutation_bytes.load(Ordering::Relaxed),
                data_page_bytes: self.data_page_bytes.load(Ordering::Relaxed),
                leaf_page_bytes: self.leaf_page_bytes.load(Ordering::Relaxed),
                branch_page_bytes: self.branch_page_bytes.load(Ordering::Relaxed),
                overflow_page_bytes: self.overflow_page_bytes.load(Ordering::Relaxed),
                write_buffer_bytes: self.write_buffer_bytes.load(Ordering::Relaxed),
                metadata_bytes: self.metadata_bytes.load(Ordering::Relaxed),
            },
            corruption_detections: self.corruption_detections.load(Ordering::Relaxed),
            checkpoint_failures: self.checkpoint_failures.load(Ordering::Relaxed),
        }
    }
}

pub(crate) struct CheckpointActivity {
    metrics: Arc<RuntimeMetrics>,
}

impl Drop for CheckpointActivity {
    fn drop(&mut self) {
        let _ = self.metrics.checkpoints_in_progress.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| Some(current.saturating_sub(1)),
        );
    }
}

pub(crate) struct RuntimeSnapshot {
    pub(crate) commits: LatencyStats,
    pub(crate) syncs: LatencyStats,
    pub(crate) wal_preflights: LatencyStats,
    pub(crate) wal_encodes: LatencyStats,
    pub(crate) wal_writes: LatencyStats,
    pub(crate) wal_syncs: LatencyStats,
    pub(crate) wal_publications: LatencyStats,
    pub(crate) checkpoints: LatencyStats,
    pub(crate) checkpoint_starts: u64,
    pub(crate) checkpoints_in_progress: u64,
    pub(crate) checkpoint_bytes: u64,
    pub(crate) writes: WriteStats,
    pub(crate) corruption_detections: u64,
    pub(crate) checkpoint_failures: u64,
}

fn add_saturating(counter: &AtomicU64, amount: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(amount))
    });
}

struct TimingMetrics {
    operations: AtomicU64,
    total_nanos: AtomicU64,
    last_nanos: AtomicU64,
    max_nanos: AtomicU64,
    histogram: [AtomicU64; HISTOGRAM_BUCKETS],
}

impl Default for TimingMetrics {
    fn default() -> Self {
        Self {
            operations: AtomicU64::new(0),
            total_nanos: AtomicU64::new(0),
            last_nanos: AtomicU64::new(0),
            max_nanos: AtomicU64::new(0),
            histogram: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl TimingMetrics {
    fn record(&self, duration: Duration) {
        let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        self.histogram[histogram_index(nanos)].fetch_add(1, Ordering::Relaxed);
        self.operations.fetch_add(1, Ordering::Relaxed);
        self.total_nanos.fetch_add(nanos, Ordering::Relaxed);
        self.last_nanos.store(nanos, Ordering::Relaxed);
        self.max_nanos.fetch_max(nanos, Ordering::Relaxed);
    }

    fn snapshot(&self) -> LatencyStats {
        LatencyStats {
            operations: self.operations.load(Ordering::Relaxed),
            total_nanos: self.total_nanos.load(Ordering::Relaxed),
            last_nanos: self.last_nanos.load(Ordering::Relaxed),
            max_nanos: self.max_nanos.load(Ordering::Relaxed),
            p50_nanos: self.percentile(50, 100),
            p95_nanos: self.percentile(95, 100),
            p99_nanos: self.percentile(99, 100),
            p999_nanos: self.percentile(999, 1_000),
        }
    }

    fn percentile(&self, numerator: u64, denominator: u64) -> u64 {
        let total = self.histogram.iter().fold(0_u64, |count, bucket| {
            count.saturating_add(bucket.load(Ordering::Relaxed))
        });
        if total == 0 {
            return 0;
        }
        let rank = (u128::from(total) * u128::from(numerator)).div_ceil(u128::from(denominator));
        let mut cumulative = 0_u128;
        for (index, bucket) in self.histogram.iter().enumerate() {
            cumulative = cumulative.saturating_add(u128::from(bucket.load(Ordering::Relaxed)));
            if cumulative >= rank {
                return histogram_upper_bound(index);
            }
        }
        u64::MAX
    }
}

fn histogram_index(nanos: u64) -> usize {
    if nanos < HISTOGRAM_EXACT_BUCKETS as u64 {
        return nanos as usize;
    }
    let exponent = u64::BITS - 1 - nanos.leading_zeros();
    let shift = exponent - HISTOGRAM_SUB_BUCKET_BITS;
    let sub_bucket = ((nanos >> shift) & (HISTOGRAM_SUB_BUCKETS as u64 - 1)) as usize;
    HISTOGRAM_EXACT_BUCKETS
        + (exponent as usize - HISTOGRAM_SUB_BUCKET_BITS as usize) * HISTOGRAM_SUB_BUCKETS
        + sub_bucket
}

fn histogram_upper_bound(index: usize) -> u64 {
    if index < HISTOGRAM_EXACT_BUCKETS {
        return index as u64;
    }
    let relative = index - HISTOGRAM_EXACT_BUCKETS;
    let exponent = HISTOGRAM_SUB_BUCKET_BITS as usize + relative / HISTOGRAM_SUB_BUCKETS;
    let sub_bucket = relative % HISTOGRAM_SUB_BUCKETS;
    let width = 1_u64 << (exponent - HISTOGRAM_SUB_BUCKET_BITS as usize);
    let lower = (1_u64 << exponent).saturating_add((sub_bucket as u64).saturating_mul(width));
    lower.saturating_add(width - 1)
}

#[cfg(test)]
mod tests {
    use super::{
        HISTOGRAM_BUCKETS, RuntimeMetrics, TimingMetrics, histogram_index, histogram_upper_bound,
    };
    use std::mem::size_of;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn histogram_bounds_cover_the_complete_duration_domain() {
        let samples = [
            0,
            1,
            15,
            16,
            17,
            31,
            32,
            1_000,
            1_000_000,
            1_u64 << 63,
            u64::MAX,
        ];
        for sample in samples {
            let index = histogram_index(sample);
            assert!(index < HISTOGRAM_BUCKETS);
            let upper = histogram_upper_bound(index);
            assert!(upper >= sample, "{sample} mapped below its value");
            if sample > 15 {
                assert!(
                    u128::from(upper - sample) * 16 <= u128::from(sample),
                    "{sample} exceeded the histogram error bound at {upper}"
                );
            } else {
                assert_eq!(upper, sample);
            }
        }
    }

    #[test]
    fn percentile_snapshot_uses_nearest_rank_upper_bounds() {
        let metrics = TimingMetrics::default();
        for nanos in 1..=1_000 {
            metrics.record(Duration::from_nanos(nanos));
        }

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.operations, 1_000);
        for (estimate, exact) in [
            (snapshot.p50_nanos, 500),
            (snapshot.p95_nanos, 950),
            (snapshot.p99_nanos, 990),
            (snapshot.p999_nanos, 999),
        ] {
            assert!(estimate >= exact);
            assert!(u128::from(estimate - exact) * 16 <= u128::from(exact));
        }
    }

    #[test]
    fn empty_histogram_reports_zero_percentiles() {
        let snapshot = TimingMetrics::default().snapshot();
        assert_eq!(snapshot.p50_nanos, 0);
        assert_eq!(snapshot.p95_nanos, 0);
        assert_eq!(snapshot.p99_nanos, 0);
        assert_eq!(snapshot.p999_nanos, 0);
    }

    #[test]
    fn all_runtime_histograms_have_a_bounded_footprint() {
        assert!(size_of::<RuntimeMetrics>() <= 64 * 1024);
    }

    #[test]
    fn checkpoint_activity_tracks_started_and_active_work() {
        let metrics = Arc::new(RuntimeMetrics::default());
        let activity = metrics.checkpoint_activity();
        let active = metrics.checkpoint_stats();
        assert_eq!(active.started, 1);
        assert_eq!(active.in_progress, 1);

        drop(activity);
        let complete = metrics.checkpoint_stats();
        assert_eq!(complete.started, 1);
        assert_eq!(complete.in_progress, 0);
    }
}
