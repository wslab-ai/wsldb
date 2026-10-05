use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const CRASH_COVERAGE: &[(&str, &str)] = &[
    ("backup_sync", "maintenance_crash.rs"),
    ("buffered_compaction_publish", "compaction_race.rs"),
    ("checkpoint_data_sync", "buffer_checkpoint_crash.rs"),
    ("checkpoint_metadata_sync", "buffer_checkpoint_crash.rs"),
    ("checkpoint_metadata_write", "buffer_checkpoint_crash.rs"),
    ("checkpoint_stream_write", "buffer_checkpoint_crash.rs"),
    ("checkpoint_wal_clear", "buffer_checkpoint_crash.rs"),
    ("compaction_data_sync", "maintenance_crash.rs"),
    ("compaction_final_sync", "maintenance_crash.rs"),
    ("compaction_metadata_sync", "maintenance_crash.rs"),
    ("compaction_metadata_write", "maintenance_crash.rs"),
    ("compaction_page_write", "maintenance_crash.rs"),
    ("compaction_ready", "operations.rs"),
    ("compaction_rename", "operations.rs"),
    ("compaction_truncate", "maintenance_crash.rs"),
    (
        "compaction_wal_handoff_directory_sync",
        "compaction_race.rs",
    ),
    ("compaction_wal_handoff_rename", "compaction_race.rs"),
    ("compaction_wal_handoff_sync", "compaction_race.rs"),
    ("compaction_wal_handoff_write", "compaction_race.rs"),
    ("compaction_wal_stage_sync", "compaction_race.rs"),
    ("data_sync", "append.rs"),
    ("initial_metadata_sync", "initialization.rs"),
    ("initial_metadata_write", "initialization.rs"),
    ("metadata_sync", "append.rs"),
    ("metadata_write", "append.rs"),
    ("page_write", "append.rs"),
    ("snapshot_cleanup_directory_sync", "maintenance_crash.rs"),
    ("snapshot_copy_write", "maintenance_crash.rs"),
    ("snapshot_publish_directory_sync", "maintenance_crash.rs"),
    ("snapshot_publish_link", "maintenance_crash.rs"),
    ("snapshot_temporary_remove", "maintenance_crash.rs"),
    ("wal_truncate", "buffer_checkpoint_crash.rs"),
    (
        "write_buffer_snapshot_directory_sync",
        "write_buffer_snapshot_crash.rs",
    ),
    (
        "write_buffer_snapshot_rename",
        "write_buffer_snapshot_crash.rs",
    ),
    (
        "write_buffer_snapshot_sync",
        "write_buffer_snapshot_crash.rs",
    ),
    (
        "write_buffer_snapshot_write",
        "write_buffer_snapshot_crash.rs",
    ),
    ("write_buffer_sync", "group_commit.rs"),
    ("write_buffer_write", "group_commit.rs"),
];

const FAULT_COVERAGE: &[(&str, &str)] = &[
    ("active_replace", "maintenance_failures.rs"),
    ("active_replace_directory_sync", "maintenance_failures.rs"),
    ("backup_sync", "maintenance_failures.rs"),
    ("checkpoint_data_sync", "maintenance_failures.rs"),
    ("checkpoint_metadata_sync", "maintenance_failures.rs"),
    ("checkpoint_metadata_write", "maintenance_failures.rs"),
    ("checkpoint_stream_write", "maintenance_failures.rs"),
    ("checkpoint_wal_clear", "maintenance_failures.rs"),
    ("compaction_data_sync", "maintenance_failures.rs"),
    ("compaction_final_sync", "maintenance_failures.rs"),
    ("compaction_metadata_sync", "maintenance_failures.rs"),
    ("compaction_metadata_write", "maintenance_failures.rs"),
    ("compaction_truncate", "maintenance_failures.rs"),
    (
        "compaction_wal_handoff_directory_sync",
        "compaction_race.rs",
    ),
    ("compaction_wal_handoff_rename", "compaction_race.rs"),
    ("compaction_wal_handoff_sync", "compaction_race.rs"),
    ("compaction_wal_handoff_write", "compaction_race.rs"),
    ("compaction_wal_stage_sync", "compaction_race.rs"),
    ("data_sync", "io_failures.rs"),
    ("initial_metadata_sync", "initialization.rs"),
    ("metadata_sync", "io_failures.rs"),
    ("positioned_write", "io_failures.rs"),
    ("snapshot_cleanup_directory_sync", "maintenance_failures.rs"),
    ("snapshot_publish_directory_sync", "maintenance_failures.rs"),
    ("snapshot_publish_link", "maintenance_failures.rs"),
    ("snapshot_temporary_remove", "maintenance_failures.rs"),
    ("wal_sidecar_directory_sync", "io_failures.rs"),
    ("wal_truncate_sync", "maintenance_failures.rs"),
    (
        "write_buffer_snapshot_directory_sync",
        "maintenance_failures.rs",
    ),
    ("write_buffer_snapshot_rename", "maintenance_failures.rs"),
    ("write_buffer_snapshot_sync", "maintenance_failures.rs"),
    ("write_buffer_sync", "io_failures.rs"),
];

#[test]
fn every_crash_boundary_has_an_explicit_test_owner() {
    let root = repository_root();
    let actual = source_stages(&root, "crash_injection::hit(", false);
    assert_coverage(&root, "crash", "WSLDB_FAILPOINT", actual, CRASH_COVERAGE);
}

#[test]
fn every_fault_boundary_has_an_explicit_test_owner() {
    let root = repository_root();
    let mut actual = source_stages(&root, "fault_injection::before(", false);
    actual.extend(source_stages(&root, "stage_selected(", true));
    assert_coverage(&root, "fault", "WSLDB_IO_FAILPOINT", actual, FAULT_COVERAGE);
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source_stages(root: &Path, marker: &str, allow_dynamic: bool) -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let mut stages = BTreeSet::new();
    for path in files {
        let source = fs::read_to_string(&path).expect("production source must be readable");
        let mut remaining = source.as_str();
        while let Some(position) = remaining.find(marker) {
            let after_marker = remaining[position + marker.len()..].trim_start();
            if let Some(after_quote) = after_marker.strip_prefix('"') {
                let end = after_quote
                    .find('"')
                    .expect("injection stage string must be terminated");
                let stage = &after_quote[..end];
                assert!(
                    stage
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
                    "invalid injection stage {stage:?} in {}",
                    path.display()
                );
                stages.insert(stage.to_owned());
            } else if !allow_dynamic {
                panic!("{marker} must use a string literal in {}", path.display());
            }
            remaining = &after_marker[1..];
        }
    }
    stages
}

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory).expect("source directory must be readable");
    for entry in entries {
        let path = entry.expect("source entry must be readable").path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn assert_coverage(
    root: &Path,
    kind: &str,
    environment: &str,
    actual: BTreeSet<String>,
    coverage: &[(&str, &str)],
) {
    let declared: BTreeMap<_, _> = coverage.iter().copied().collect();
    assert_eq!(
        declared.len(),
        coverage.len(),
        "duplicate {kind} injection coverage entry"
    );
    let expected: BTreeSet<String> = declared.keys().map(|stage| (*stage).to_owned()).collect();
    assert_eq!(actual, expected, "{kind} injection coverage changed");

    for (stage, test_file) in declared {
        let path = root.join("tests").join(test_file);
        let test = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("could not read {} for {stage}: {error}", path.display())
        });
        assert!(
            test.contains(&format!("\"{stage}\"")),
            "{kind} stage {stage} is not named in {}",
            path.display()
        );
        assert!(
            test.contains(environment),
            "{} does not configure {environment} for {kind} stage {stage}",
            path.display()
        );
    }
}
