use std::path::Path;
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

pub const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);

pub fn wait_for_path(child: &mut Child, path: &Path, context: &str) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "{context} exited before publishing its ready marker"
        );
        assert!(
            started.elapsed() < PROCESS_TIMEOUT,
            "{context} did not publish its ready marker"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

pub fn wait_for_success(child: &mut Child, context: &str) {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{context} failed");
            return;
        }
        if started.elapsed() >= PROCESS_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{context} did not finish");
        }
        thread::sleep(Duration::from_millis(1));
    }
}
