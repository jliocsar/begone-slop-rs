//! Which files a run covers (git's view of the tree when there is one, a walk otherwise), and
//! which project they belong to.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use toml::{Table, Value};

const SKIPPED_DIRECTORIES: [&str; 1] = ["target"];

/// The glob component that matches any number of directories.
const ANY_DEPTH: &str = "**";

/// Every Rust file under `paths`. Inside a git checkout, ignored files are left out.
///
/// # Errors
///
/// Fails when a path doesn't exist, or a directory can't be read during the fallback walk.
pub fn rust_files(paths: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();

    for path in paths {
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} doesn't exist", path.display()),
            ));
        }

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

/// The project `start` (an absolute directory) belongs to: the workspace whose members include
/// the nearest package, else that package, else `start` itself. Config, drift and boundaries
/// are read from here, wherever in the project `cargo bs` runs.
#[must_use]
pub fn project_root(start: &Path) -> PathBuf {
    let Some(package) = start
        .ancestors()
        .find(|directory| directory.join("Cargo.toml").is_file())
    else {
        return start.to_owned();
    };

    package
        .ancestors()
        .find(|directory| {
            let workspace = fs::read_to_string(directory.join("Cargo.toml"))
                .ok()
                .and_then(|text| text.parse::<Table>().ok())
                .and_then(|manifest| manifest.get("workspace").cloned());

            let Some(workspace) = workspace else {
                return false;
            };

            let relative = package.strip_prefix(directory).unwrap_or(package);

            let listed = |key: &str| {
                workspace
                    .get(key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .any(|pattern| glob_matches(pattern, relative))
            };

            relative.as_os_str().is_empty() || (listed("members") && !listed("exclude"))
        })
        .unwrap_or(package)
        .to_owned()
}

/// A workspace `members` pattern against a relative directory: `*` within one component,
/// `**` across any number of them.
#[must_use]
pub fn glob_matches(pattern: &str, path: &Path) -> bool {
    let parts: Vec<&str> = pattern
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    let names: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();

    components_match(&parts, &names)
}

fn components_match(parts: &[&str], names: &[String]) -> bool {
    let Some((part, rest)) = parts.split_first() else {
        return names.is_empty();
    };

    if *part == ANY_DEPTH {
        return components_match(rest, names)
            || names
                .split_first()
                .is_some_and(|(_, after)| components_match(parts, after));
    }

    names
        .split_first()
        .is_some_and(|(name, after)| wildcard_matches(part, name) && components_match(rest, after))
}

/// `*` matches any run of characters; everything else matches itself.
fn wildcard_matches(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((prefix, rest)) => name.strip_prefix(prefix).is_some_and(|tail| {
            (0..=tail.len())
                .filter(|start| tail.is_char_boundary(*start))
                .any(|start| {
                    tail.get(start..)
                        .is_some_and(|suffix| wildcard_matches(rest, suffix))
                })
        }),
    }
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::glob_matches;

    #[test]
    fn member_patterns_match_by_component() {
        assert!(glob_matches("crates/*", Path::new("crates/core")));
        assert!(glob_matches("crates/rex-*", Path::new("crates/rex-client")));
        assert!(glob_matches("tools/**", Path::new("tools/a/b")));
        assert!(!glob_matches("crates/*", Path::new("crates/core/nested")));
        assert!(!glob_matches("crates/*", Path::new("fixtures/demo")));
    }
}
