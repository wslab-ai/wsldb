use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);
pub struct TestFile {
    path: PathBuf,
}

impl TestFile {
    pub fn new(label: &str) -> Self {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let name = format!("wsldb-{}-{sequence}-{label}.wsldb", std::process::id());
        Self {
            path: std::env::temp_dir().join(name),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestFile {
    fn drop(&mut self) {
        remove_if_present(&self.path);
        remove_replacements_for(&self.path);
        remove_temporaries_for(&self.path);
        for suffix in [".lock", ".wal"] {
            let mut sidecar = self.path.as_os_str().to_owned();
            sidecar.push(suffix);
            let sidecar = Path::new(&sidecar);
            remove_if_present(sidecar);
            remove_replacements_for(sidecar);
            remove_temporaries_for(sidecar);
            remove_files_with_prefix(sidecar, ".wsldb-handoff-");
        }
    }
}

fn remove_temporaries_for(path: &Path) {
    remove_files_with_prefix(path, ".wsldb-tmp-");
}

fn remove_replacements_for(path: &Path) {
    remove_files_with_prefix(path, ".wsldb-replacement-");
}

fn remove_files_with_prefix(path: &Path, suffix: &str) {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let prefix = format!("{file_name}{suffix}");
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let is_replacement = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(&prefix));
        if is_replacement && entry.file_type().is_ok_and(|kind| kind.is_file()) {
            remove_if_present(&entry.path());
        }
    }
}

fn remove_if_present(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("failed to clean up {}: {error}", path.display()),
    }
}
