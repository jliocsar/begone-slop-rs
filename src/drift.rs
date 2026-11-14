//! `preset-drift`: the project still runs the preset it was given.
//!
//! The quickest way past a lint gate is to turn the lint down, so `cargo bs` checks the gate
//! itself. Every preset lint must stay at its preset level or stricter, every crate must
//! inherit the workspace lints, and every `clippy.toml` keeps the preset's settings. A project
//! may add lints and settings of its own; it may not loosen these. There's no waiver.

use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::files;
use crate::init::{CLIPPY, LINTS};

const LEVELS: [&str; 4] = ["allow", "warn", "deny", "forbid"];
const CLIPPY_FILES: [&str; 2] = ["clippy.toml", ".clippy.toml"];
const WORKSPACE_LINTS: [&str; 2] = ["workspace", "lints"];

/// One way the project loosened the preset.
#[derive(Debug)]
pub struct Drift {
    pub path: PathBuf,
    pub line: usize,
    pub message: String,
}

/// A manifest's lints: its `[workspace.lints]` when it has them, else its own `[lints]`.
struct Manifest {
    path: PathBuf,
    text: String,
    table: Table,
}

impl Manifest {
    fn read(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        let table = text.parse::<Table>().ok()?;

        Some(Self {
            path: path.to_owned(),
            text,
            table,
        })
    }

    fn table_at(&self, keys: &[&str]) -> Option<&Table> {
        keys.iter()
            .try_fold(&self.table, |table, key| table.get(*key)?.as_table())
    }

    fn line_of(&self, needle: &str) -> usize {
        self.text
            .lines()
            .position(|line| line.trim_start().starts_with(needle))
            .map_or(1, |index| index + 1)
    }

    fn drift(&self, needle: &str, message: String) -> Drift {
        Drift {
            path: self.path.clone(),
            line: self.line_of(needle),
            message,
        }
    }
}

/// Checks the manifests and clippy configs under `root` against the preset.
#[must_use]
pub fn check(root: &Path) -> Vec<Drift> {
    let Some(root_manifest) = Manifest::read(&root.join("Cargo.toml")) else {
        return Vec::new();
    };

    let Ok(preset) = LINTS.parse::<Table>() else {
        return Vec::new();
    };

    let workspace_lints = root_manifest.table_at(&WORKSPACE_LINTS).is_some();

    let lints_at: &[&str] = if workspace_lints {
        &WORKSPACE_LINTS
    } else {
        &["lints"]
    };

    let mut drifts = lint_levels(&root_manifest, &preset, lints_at);
    let manifests = files::named(root, &["Cargo.toml"]);
    drifts.extend(
        manifests
            .iter()
            .filter_map(|path| Manifest::read(path))
            .filter(|manifest| workspace_lints && manifest.table.contains_key("package"))
            .filter(|manifest| {
                manifest
                    .table_at(&["lints"])
                    .and_then(|lints| lints.get("workspace"))
                    .and_then(Value::as_bool)
                    != Some(true)
            })
            .map(|manifest| {
                manifest.drift(
                    "[lints]",
                    "this crate doesn't inherit the workspace lints; add `[lints] workspace = true`".to_owned(),
                )
            }),
    );
    drifts.extend(clippy_settings(root));

    drifts
}

fn lint_levels(manifest: &Manifest, preset: &Table, prefix: &[&str]) -> Vec<Drift> {
    let Some(preset_lints) = preset
        .get("workspace")
        .and_then(|workspace| workspace.get("lints"))
        .and_then(Value::as_table)
    else {
        return Vec::new();
    };

    preset_lints
        .iter()
        .flat_map(|(tool, lints)| {
            let project = manifest.table_at(&[prefix, &[tool.as_str()]].concat());

            lints
                .as_table()
                .into_iter()
                .flatten()
                .filter_map(move |(lint, wanted)| {
                    let wanted = level_of(wanted)?;
                    let have = project.and_then(|table| table.get(lint)).and_then(level_of);

                    match have {
                        Some(have) if have >= wanted => None,
                        Some(have) => Some(manifest.drift(
                            &format!("{lint} ="),
                            format!(
                                "`{tool}::{lint}` is turned down to `{}`; the preset sets `{}`",
                                LEVELS.get(have).unwrap_or(&"?"),
                                LEVELS.get(wanted).unwrap_or(&"?")
                            ),
                        )),
                        None => Some(manifest.drift(
                            "[workspace.lints",
                            format!("`{tool}::{lint}` is missing from the lints table; `cargo bs init` prints the preset"),
                        )),
                    }
                })
        })
        .collect()
}

fn clippy_settings(root: &Path) -> Vec<Drift> {
    let Ok(preset) = CLIPPY.parse::<Table>() else {
        return Vec::new();
    };

    let configs = files::named(root, &CLIPPY_FILES);

    if configs.is_empty() {
        return vec![Drift {
            path: root.join("clippy.toml"),
            line: 1,
            message: "there's no clippy.toml, so clippy runs without the preset's settings; run `cargo bs init`".to_owned(),
        }];
    }

    configs
        .iter()
        .filter_map(|path| Manifest::read(path))
        .flat_map(|config| {
            preset
                .iter()
                .filter(|(key, wanted)| config.table.get(*key) != Some(*wanted))
                .map(|(key, wanted)| {
                    config.drift(
                        &format!("{key} ="),
                        format!("`{key}` must stay `{wanted}`, as the preset sets it"),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `"warn"` or `{ level = "warn", priority = -1 }`, as an index into [`LEVELS`].
fn level_of(value: &Value) -> Option<usize> {
    let level = match value {
        Value::String(level) => level.as_str(),
        Value::Table(table) => table.get("level")?.as_str()?,
        _ => return None,
    };

    LEVELS.iter().position(|known| *known == level)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::check;

    fn drifts_in(case: &str, manifest: &str, clippy: &str) -> Vec<String> {
        let directory = std::env::temp_dir().join(format!("cargo-bs-drift-{case}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("Cargo.toml"), manifest).unwrap();
        fs::write(directory.join("clippy.toml"), clippy).unwrap();
        let messages = check(&directory)
            .into_iter()
            .map(|drift| drift.message)
            .collect();
        fs::remove_dir_all(&directory).unwrap();

        messages
    }

    #[test]
    fn the_preset_itself_passes() {
        let manifest = format!(
            "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n\n{}",
            crate::init::LINTS
        );

        assert_eq!(
            drifts_in("preset", &manifest, crate::init::CLIPPY),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_lint_turned_down_is_reported() {
        let manifest = format!(
            "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n\n{}",
            crate::init::LINTS
        )
        .replace("unwrap_used = \"warn\"", "unwrap_used = \"allow\"");
        let found = drifts_in("turned-down", &manifest, crate::init::CLIPPY);

        assert_eq!(found.len(), 1);
        assert!(found.iter().all(|message| message.contains("unwrap_used")));
    }

    #[test]
    fn a_loosened_threshold_is_reported() {
        let manifest = format!(
            "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n\n{}",
            crate::init::LINTS
        );
        let clippy = crate::init::CLIPPY.replace(
            "excessive-nesting-threshold = 5",
            "excessive-nesting-threshold = 9",
        );
        let found = drifts_in("threshold", &manifest, &clippy);

        assert_eq!(found.len(), 1);
    }

    #[test]
    fn a_crate_that_drops_the_workspace_lints_is_reported() {
        let manifest = format!("[package]\nname = \"x\"\n\n{}", crate::init::LINTS);
        let found = drifts_in("no-inherit", &manifest, crate::init::CLIPPY);

        assert_eq!(found.len(), 1);
        assert!(found.iter().all(|message| message.contains("inherit")));
    }
}
