//! Which `.rs` files a run covers: git's view of the tree when there is one, a walk otherwise.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const SKIPPED_DIRECTORIES: [&str; 1] = ["target"];

/// Every Rust file under `paths`. Inside a git checkout, ignored files are left out.
///
/// # Errors
///
/// Fails when a directory can't be read during the fallback walk.
pub fn rust_files(paths: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();

    for path in paths {
        if path.is_file() {
            found.extend(is_rust(path).then(|| path.clone()));
        } else {
            found.extend(match tracked_by_git(path) {
                Some(files) => files,
                None => walk(path)?,
            });
        }
    }

    found.sort();
    found.dedup();

    Ok(found)
}

fn is_rust(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "rs")
}

fn tracked_by_git(directory: &Path) -> Option<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "*.rs",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let listing = String::from_utf8(output.stdout).ok()?;

    Some(
        listing
            .split('\0')
            .filter(|entry| !entry.is_empty())
            .map(|entry| directory.join(entry))
            .filter(|path| path.is_file())
            .collect(),
    )
}

fn walk(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();

    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let skipped = name.starts_with('.') || SKIPPED_DIRECTORIES.contains(&name.as_ref());

        if path.is_dir() && !skipped {
            found.extend(walk(&path)?);
        } else if is_rust(&path) {
            found.push(path);
        }
    }

    Ok(found)
}
