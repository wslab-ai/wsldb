use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use wsldb::{Database, DatabaseOptions, Error};

const DEFAULT_WORKERS: u32 = 4;
const DEFAULT_COMMITS: u32 = 12;
const STRESS_WORKERS: u32 = 12;
const STRESS_COMMITS: u32 = 500;
const BUFFER_BYTES: u64 = 64 * 1024 * 1024;
const START_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);
const STRESS_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[test]
fn direct_multiprocess_writers_preserve_every_successful_commit() {
    run_parent(WriteMode::Direct);
}

#[test]
fn buffered_multiprocess_writers_preserve_every_successful_commit() {
    run_parent(WriteMode::Buffered);
}

#[test]
fn multiprocess_writer_worker() {
    let Some(path) = std::env::var_os("WSLDB_WRITER_WORKER_PATH") else {
        return;
    };
    let worker = env_u32("WSLDB_WRITER_WORKER_ID", 0);
    let commits = env_u32("WSLDB_WRITER_WORKER_COMMITS", 0);
    let mode = WriteMode::from_env();
    let ready = env_path("WSLDB_WRITER_READY_PATH");
    let start = env_path("WSLDB_WRITER_START_PATH");
    let database = mode.open(Path::new(&path)).unwrap();

    File::create(ready).unwrap().sync_all().unwrap();
    wait_for_path(&start, START_TIMEOUT);

    let mut conflicts = 0_u64;
    for operation in 0..commits {
        let entries = commit_entries(worker, operation);
        loop {
            let mut writer = database.write().unwrap();
            for (key, value) in &entries {
                writer.put(key, value).unwrap();
            }
            match writer.commit() {
                Ok(_) => break,
                Err(Error::WriteConflict) => {
                    conflicts += 1;
                    database.refresh().unwrap();
                    thread::yield_now();
                }
                Err(error) => panic!("writer process failed: {error}"),
            }
        }

        let mut aborted = database.write().unwrap();
        let key = format!("aborted-{worker:04}-{operation:08}");
        aborted.put(key.as_bytes(), b"must-not-be-visible").unwrap();
    }

    println!("WSLDB_STRESS_RESULT worker={worker} commits={commits} conflicts={conflicts}");
}

fn run_parent(mode: WriteMode) {
    if std::env::var_os("WSLDB_WRITER_WORKER_PATH").is_some() {
        return;
    }
    let stress = std::env::var_os("WSLDB_RUN_MULTIPROCESS_STRESS").is_some();
    let workers = env_u32(
        "WSLDB_MULTIPROCESS_WORKERS",
        if stress {
            STRESS_WORKERS
        } else {
            DEFAULT_WORKERS
        },
    );
    let commits = env_u32(
        "WSLDB_MULTIPROCESS_COMMITS",
        if stress {
            STRESS_COMMITS
        } else {
            DEFAULT_COMMITS
        },
    );
    assert!(workers > 1);
    assert!(commits > 0);

    let files = StressFiles::new(mode);
    drop(mode.open(&files.database).unwrap());
    let started = Instant::now();
    let children = spawn_workers(&files, mode, workers, commits);
    wait_for_workers(&files, workers);
    File::create(&files.start).unwrap().sync_all().unwrap();

    let mut conflicts = 0_u64;
    let timeout = if stress {
        STRESS_COMPLETION_TIMEOUT
    } else {
        DEFAULT_COMPLETION_TIMEOUT
    };
    for output in wait_for_children(children, timeout) {
        assert!(
            output.status.success(),
            "worker failed with {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        conflicts += parse_conflicts(&output.stdout);
    }
    assert!(
        conflicts > 0,
        "the run did not exercise stale-head rejection"
    );

    let database = Database::open(&files.database).unwrap();
    let expected = expected_entries(workers, commits);
    assert_eq!(database.verify().unwrap().entries, expected.len() as u64);
    assert_eq!(
        database.stats().unwrap().transaction_id.value(),
        u64::from(workers) * u64::from(commits)
    );
    let actual: BTreeMap<_, _> = database.read().unwrap().iter().unwrap().collect();
    assert_eq!(actual, expected);

    if mode == WriteMode::Buffered {
        database.flush().unwrap();
        drop(database);
        let reopened = Database::open(&files.database).unwrap();
        reopened.verify().unwrap();
        let actual: BTreeMap<_, _> = reopened.read().unwrap().iter().unwrap().collect();
        assert_eq!(actual, expected);
    }

    println!(
        "WSLDB_MULTIPROCESS mode={} workers={workers} commits_per_worker={commits} total_commits={} conflicts={conflicts} elapsed_ms={}",
        mode.as_str(),
        workers * commits,
        started.elapsed().as_millis()
    );
}

fn wait_for_children(mut children: Vec<Child>, timeout: Duration) -> Vec<Output> {
    let started = Instant::now();
    loop {
        let mut all_finished = true;
        for child in &mut children {
            if child.try_wait().unwrap().is_none() {
                all_finished = false;
            }
        }
        if all_finished {
            break;
        }
        if started.elapsed() >= timeout {
            for child in &mut children {
                if child.try_wait().unwrap().is_none() {
                    let _ = child.kill();
                }
            }
            for child in children {
                let _ = child.wait_with_output();
            }
            panic!("multiprocess writers exceeded {timeout:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect()
}

fn spawn_workers(files: &StressFiles, mode: WriteMode, workers: u32, commits: u32) -> Vec<Child> {
    (0..workers)
        .map(|worker| {
            Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("multiprocess_writer_worker")
                .arg("--nocapture")
                .arg("--test-threads=1")
                .env("WSLDB_WRITER_WORKER_PATH", &files.database)
                .env("WSLDB_WRITER_WORKER_ID", worker.to_string())
                .env("WSLDB_WRITER_WORKER_COMMITS", commits.to_string())
                .env("WSLDB_WRITER_MODE", mode.as_str())
                .env("WSLDB_WRITER_READY_PATH", files.ready(worker))
                .env("WSLDB_WRITER_START_PATH", &files.start)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect()
}

fn wait_for_workers(files: &StressFiles, workers: u32) {
    let started = Instant::now();
    while (0..workers).any(|worker| !files.ready(worker).exists()) {
        assert!(
            started.elapsed() < START_TIMEOUT,
            "workers did not become ready"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn wait_for_path(path: &Path, timeout: Duration) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < timeout,
            "start signal was not published"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn commit_entries(worker: u32, operation: u32) -> [(Vec<u8>, Vec<u8>); 2] {
    let prefix = format!("worker-{worker:04}-commit-{operation:08}");
    [
        (
            format!("{prefix}-marker").into_bytes(),
            format!("committed:{worker}:{operation}").into_bytes(),
        ),
        (
            format!("{prefix}-payload").into_bytes(),
            payload(worker, operation),
        ),
    ]
}

fn payload(worker: u32, operation: u32) -> Vec<u8> {
    let mut state = u64::from(worker) << 32 | u64::from(operation);
    (0..257)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

fn expected_entries(workers: u32, commits: u32) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut expected = BTreeMap::new();
    for worker in 0..workers {
        for operation in 0..commits {
            expected.extend(commit_entries(worker, operation));
        }
    }
    expected
}

fn parse_conflicts(stdout: &[u8]) -> u64 {
    String::from_utf8_lossy(stdout)
        .lines()
        .find_map(|line| {
            line.find("WSLDB_STRESS_RESULT ").and_then(|position| {
                line[position + "WSLDB_STRESS_RESULT ".len()..]
                    .split_ascii_whitespace()
                    .find_map(|field| field.strip_prefix("conflicts="))
                    .and_then(|value| value.parse().ok())
            })
        })
        .expect("worker did not report its conflict count")
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .map(|value| value.parse().expect("invalid unsigned integer"))
        .unwrap_or(default)
}

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).expect("missing worker path"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriteMode {
    Direct,
    Buffered,
}

impl WriteMode {
    fn from_env() -> Self {
        match std::env::var("WSLDB_WRITER_MODE").as_deref() {
            Ok("direct") => Self::Direct,
            Ok("buffered") => Self::Buffered,
            _ => panic!("invalid worker mode"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Buffered => "buffered",
        }
    }

    fn open(self, path: &Path) -> wsldb::Result<Database> {
        match self {
            Self::Direct => Database::open(path),
            Self::Buffered => Database::open_with_options(
                path,
                DatabaseOptions::new().write_buffer_bytes(BUFFER_BYTES),
            ),
        }
    }
}

struct StressFiles {
    directory: PathBuf,
    database: PathBuf,
    start: PathBuf,
}

impl StressFiles {
    fn new(mode: WriteMode) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "wsldb-multiprocess-{}-{nonce}-{}",
            std::process::id(),
            mode.as_str()
        ));
        fs::create_dir(&directory).unwrap();
        Self {
            database: directory.join("database.wsldb"),
            start: directory.join("start"),
            directory,
        }
    }

    fn ready(&self, worker: u32) -> PathBuf {
        self.directory.join(format!("ready-{worker:04}"))
    }
}

impl Drop for StressFiles {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.directory) {
            eprintln!("failed to clean up {}: {error}", self.directory.display());
        }
    }
}
