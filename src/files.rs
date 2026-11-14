//! Which files a run covers: git's view of the tree when there is one, a walk otherwise.

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
            found.extend(match tracked_by_git(path, Some("*.rs")) {
                Some(files) => files,
                None => walk(path, &is_rust)?,
            });
        }
    }

    found.sort();
    found.dedup();

    Ok(found)
}

/// Every file under `root` with one of these names, in git's view of the tree when there is one.
#[must_use]
pub fn named(root: &Path, names: &[&str]) -> Vec<PathBuf> {
    let has_name = |path: &Path| {
        path.file_name()
            .is_some_and(|name| names.iter().any(|wanted| name == *wanted))
    };

    let candidates = tracked_by_git(root, None)
        .or_else(|| walk(root, &has_name).ok())
        .unwrap_or_default();

    candidates
        .into_iter()
        .filter(|path| has_name(path))
        .collect()
}

fn is_rust(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "rs")
}

fn tracked_by_git(directory: &Path, pattern: Option<&str>) -> Option<Vec<PathBuf>> {
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
        ])
        .args(pattern)
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

fn walk(directory: &Path, keep: &dyn Fn(&Path) -> bool) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();

    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let skipped = name.starts_with('.') || SKIPPED_DIRECTORIES.contains(&name.as_ref());

        if path.is_dir() && !skipped {
            found.extend(walk(&path, keep)?);
        } else if keep(&path) {
            found.push(path);
        }
    }

    Ok(found)
}
