use std::io;

#[cfg(feature = "fault-injection")]
use std::io::ErrorKind;
#[cfg(feature = "fault-injection")]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[cfg(feature = "fault-injection")]
static MATCHING_HITS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "fault-injection")]
static POSITIONED_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "fault-injection")]
static POSITIONED_FAILED: AtomicBool = AtomicBool::new(false);

/// Fails the selected I/O boundary before the real operation runs.
#[inline]
pub(crate) fn before(stage: &str) -> io::Result<()> {
    #[cfg(feature = "fault-injection")]
    {
        if !stage_selected(stage) {
            return Ok(());
        }
        let target = configured_u64("WSLDB_IO_FAIL_AFTER", 1);
        let hit = MATCHING_HITS.fetch_add(1, Ordering::Relaxed) + 1;
        if hit == target {
            return Err(injected_error(stage));
        }
    }

    #[cfg(not(feature = "fault-injection"))]
    let _ = stage;
    Ok(())
}

/// Limits a positioned write so the next loop iteration returns an error.
#[inline]
pub(crate) fn positioned_write_len(requested: usize) -> io::Result<usize> {
    #[cfg(feature = "fault-injection")]
    {
        if !stage_selected("positioned_write") {
            return Ok(requested);
        }
        let fail_after = configured_u64("WSLDB_IO_FAIL_AFTER_BYTES", 0);
        let written = POSITIONED_BYTES.load(Ordering::Relaxed);
        if written >= fail_after {
            if !POSITIONED_FAILED.swap(true, Ordering::Relaxed) {
                return Err(injected_error("positioned_write"));
            }
            return Ok(requested);
        }
        let remaining = usize::try_from(fail_after - written).unwrap_or(usize::MAX);
        Ok(requested.min(remaining))
    }

    #[cfg(not(feature = "fault-injection"))]
    Ok(requested)
}

#[inline]
pub(crate) fn record_positioned_write(written: usize) {
    #[cfg(feature = "fault-injection")]
    if stage_selected("positioned_write") {
        let written = u64::try_from(written).unwrap_or(u64::MAX);
        POSITIONED_BYTES.fetch_add(written, Ordering::Relaxed);
    }

    #[cfg(not(feature = "fault-injection"))]
    let _ = written;
}

#[cfg(feature = "fault-injection")]
fn stage_selected(stage: &str) -> bool {
    std::env::var_os("WSLDB_IO_FAILPOINT").is_some_and(|selected| selected == stage)
}

#[cfg(feature = "fault-injection")]
fn configured_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[cfg(feature = "fault-injection")]
fn injected_error(stage: &str) -> io::Error {
    io::Error::new(
        ErrorKind::StorageFull,
        format!("injected storage failure at {stage}"),
    )
}
