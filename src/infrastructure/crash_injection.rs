#[cfg(feature = "crash-injection")]
use std::fs::File;
#[cfg(feature = "crash-injection")]
use std::path::Path;
#[cfg(feature = "crash-injection")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "crash-injection")]
use std::thread;
#[cfg(feature = "crash-injection")]
use std::time::{Duration, Instant};

#[cfg(feature = "crash-injection")]
static MATCHING_HITS: AtomicU64 = AtomicU64::new(0);

#[inline]
pub(crate) fn hit(stage: &str) {
    #[cfg(feature = "crash-injection")]
    {
        pause_if_selected(stage);
        if std::env::var_os("WSLDB_PANICPOINT").is_some_and(|selected| selected == stage) {
            panic!("injected panic at {stage}");
        }
        let Some(configured_stage) = std::env::var_os("WSLDB_FAILPOINT") else {
            return;
        };
        if configured_stage != stage {
            return;
        }
        let target = std::env::var("WSLDB_FAIL_AFTER")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1);
        let hit = MATCHING_HITS.fetch_add(1, Ordering::Relaxed) + 1;
        if hit == target {
            std::process::abort();
        }
    }

    #[cfg(not(feature = "crash-injection"))]
    let _ = stage;
}

#[cfg(feature = "crash-injection")]
fn pause_if_selected(stage: &str) {
    if std::env::var_os("WSLDB_PAUSEPOINT").is_none_or(|selected| selected != stage) {
        return;
    }
    let (Some(ready), Some(proceed)) = (
        std::env::var_os("WSLDB_PAUSE_READY"),
        std::env::var_os("WSLDB_PAUSE_CONTINUE"),
    ) else {
        std::process::abort();
    };
    let Ok(ready_file) = File::create(ready) else {
        std::process::abort();
    };
    if ready_file.sync_all().is_err() {
        std::process::abort();
    }

    let started = Instant::now();
    while !Path::new(&proceed).exists() {
        if started.elapsed() >= Duration::from_secs(30) {
            std::process::abort();
        }
        thread::sleep(Duration::from_millis(1));
    }
}
