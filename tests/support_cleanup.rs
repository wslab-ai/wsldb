mod support;

use std::fs::File;
use std::path::{Path, PathBuf};

use support::TestFile;

#[test]
fn test_file_drop_removes_main_and_wal_replacements() {
    let file = TestFile::new("replacement-cleanup");
    let main_replacement = replacement(file.path(), ".wsldb-replacement-1-0");
    let wal_replacement = replacement(file.path(), ".wal.wsldb-replacement-1-0");
    let wal_handoff = replacement(file.path(), ".wal.wsldb-handoff-1-0");
    File::create(&main_replacement).unwrap();
    File::create(&wal_replacement).unwrap();
    File::create(&wal_handoff).unwrap();

    drop(file);

    assert!(!main_replacement.exists());
    assert!(!wal_replacement.exists());
    assert!(!wal_handoff.exists());
}

fn replacement(path: &Path, suffix: &str) -> PathBuf {
    let mut replacement = path.as_os_str().to_owned();
    replacement.push(suffix);
    replacement.into()
}
