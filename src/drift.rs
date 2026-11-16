//! `preset-drift` and `cargo-config-drift`: the project still runs the gate it was given.
//!
//! The quickest way past a lint gate is to turn the lint down, so `cargo bs` checks the gate
//! itself. Every lint the preset turns on must end up at the preset's level or stricter, the
//! way Cargo and rustc resolve the table (groups and priorities included); every crate
//! inherits the workspace lints; every `clippy.toml` setting stays as strict; and nothing in
//! Cargo's own config or the environment turns lints down. A project may tighten all of it.
//! There's no waiver, and a file that can't be read counts against the project.

mod cargo_config;
mod settings;

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use self::cargo_config::Bypass;
use crate::lints::{self, Entry, Level};
use crate::{files, preset};

const CLIPPY_FILES: [&str; 2] = ["clippy.toml", ".clippy.toml"];
const CARGO_CONFIGS: [&str; 2] = [".cargo/config.toml", ".cargo/config"];

/// How many lint names a message lists before it says "and N more".
const NAMES_SHOWN: usize = 6;

/// Which half of the check found a drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    CargoConfig,
    Preset,
}

/// One way the project loosened the gate. No line means the finding isn't in a file.
#[derive(Debug)]
pub struct Drift {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub gate: Gate,
    pub message: String,
}

/// A lint the project leaves below the preset's level.
struct Lowered {
    lint: String,
    wanted: Level,
    have: Option<Level>,
}

/// A TOML file that parsed, with its text for line numbers.
struct Document {
    path: PathBuf,
    text: String,
    table: Table,
}

impl Gate {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::CargoConfig => "cargo-config-drift",
            Self::Preset => "preset-drift",
        }
    }
}

impl fmt::Display for Drift {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.path.display())?;

        if let Some(line) = self.line {
            write!(formatter, ":{line}")?;
        }

        let gate = self.gate.name();

        write!(formatter, ": {gate}: {}", self.message)
    }
}

impl Document {
    /// Reads and parses a TOML file; a failure is the drift to report.
    fn read(path: &Path, gate: Gate) -> Result<Self, Drift> {
        let unreadable = |message: String| Drift {
            path: path.to_owned(),
            line: Some(1),
            gate,
            message,
        };

        let text = fs::read_to_string(path).map_err(|cause| {
            unreadable(format!(
                "can't read it ({cause}), so the gate can't be checked"
            ))
        })?;
        let table = text.parse::<Table>().map_err(|cause| {
            let reason = cause.message();

            unreadable(format!(
                "isn't valid TOML ({reason}), so the gate can't be checked"
            ))
        })?;

        Ok(Self {
            path: path.to_owned(),
            text,
            table,
        })
    }

    fn table_at(&self, keys: &[&str]) -> Option<&Table> {
        keys.iter()
            .try_fold(&self.table, |table, key| table.get(*key)?.as_table())
    }

    /// The first line that sets `key` (`key =`, `key=` or `"key" =`), else the header line.
    fn line_of(&self, key: &str, header: &str) -> usize {
        let sets_key = |line: &str| {
            let trimmed = line.trim_start().trim_start_matches('"');

            trimmed
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start_matches('"').trim_start().starts_with('='))
        };

        self.text
            .lines()
            .position(sets_key)
            .or_else(|| {
                self.text
                    .lines()
                    .position(|line| line.trim_start().starts_with(header))
            })
            .map_or(1, |index| index + 1)
    }

    fn drift(&self, gate: Gate, line: usize, message: String) -> Drift {
        Drift {
            path: self.path.clone(),
            line: Some(line),
            gate,
            message,
        }
    }
}

/// Checks the manifests, clippy configs, Cargo configs and environment of the project at
/// `root` (its workspace root) against the preset.
#[must_use]
pub fn check(root: &Path) -> Vec<Drift> {
    let (preset_entries, preset_settings) = match (
        preset::lint_entries(),
        preset::clippy_settings(),
    ) {
        (Ok(entries), Ok(settings)) => (entries, settings),
        (Err(cause), _) | (_, Err(cause)) => {
            return vec![Drift {
                path: PathBuf::from("cargo-bs"),
                line: None,
                gate: Gate::Preset,
                message: format!(
                    "the preset built into cargo bs doesn't parse ({cause}); that's a cargo bs bug"
                ),
            }];
        }
    };

    let variables: Vec<(String, String)> = std::env::vars().collect();

    let mut drifts = manifests(root, &preset_entries);

    drifts.extend(clippy_configs(root, &preset_settings));
    drifts.extend(cargo_configs(root, &preset_entries));
    drifts.extend(
        cargo_config::environment_bypasses(&variables, &preset_entries)
            .into_iter()
            .map(|bypass| Drift {
                path: PathBuf::from(format!("${}", bypass.key)),
                line: None,
                gate: Gate::CargoConfig,
                message: bypass.message,
            }),
    );

    drifts
}

fn manifests(root: &Path, preset_entries: &[Entry]) -> Vec<Drift> {
    let manifest = match Document::read(&root.join("Cargo.toml"), Gate::Preset) {
        Ok(manifest) => manifest,
        Err(drift) => return vec![drift],
    };

    let workspace = manifest.table_at(&["workspace"]).is_some();

    let (lints_at, header): (&[&str], &str) = if workspace {
        (&["workspace", "lints"], "[workspace.lints")
    } else {
        (&["lints"], "[lints")
    };

    let empty = Table::new();
    let project = manifest.table_at(lints_at).unwrap_or(&empty);
    let mut drifts = lint_levels(&manifest, project, header, preset_entries);

    if workspace {
        drifts.extend(members_that_dont_inherit(root, &manifest));
    }

    drifts
}

/// Every lint the preset turns on, compared with where the project's table leaves it. One
/// drift per entry that turned something down, so `style = "allow"` reads as one finding.
fn lint_levels(
    manifest: &Document,
    project: &Table,
    header: &str,
    preset_entries: &[Entry],
) -> Vec<Drift> {
    let (entries, unreadable) = lints::entries(project);
    let mut drifts = entry_problems(manifest, &entries, &unreadable, header, preset_entries);

    for (culprit, lowered) in turned_down(&entries, preset_entries) {
        let entry = culprit.and_then(|name| entries.iter().find(|entry| entry.name == name));
        let line = entry.map_or_else(
            || manifest.line_of(header, header),
            |entry| line_of_entry(manifest, entry, header),
        );

        drifts.push(manifest.drift(Gate::Preset, line, lowered_message(entry, &lowered)));
    }

    drifts
}

/// Entries that can't be read, `warnings = "allow"`, and allows of lints nobody knows.
fn entry_problems(
    manifest: &Document,
    entries: &[Entry],
    unreadable: &[String],
    header: &str,
    preset_entries: &[Entry],
) -> Vec<Drift> {
    let mut drifts: Vec<Drift> = unreadable
        .iter()
        .map(|name| {
            let key = name.rsplit("::").next().unwrap_or(name);

            manifest.drift(Gate::Preset, manifest.line_of(key, header), format!("can't read `{name}`; a lint is set to \"allow\", \"warn\", \"deny\" or \"forbid\", or `{{ level = \"...\", priority = N }}`"))
        })
        .collect();

    for entry in entries.iter().filter(|entry| entry.level == Level::Allow) {
        let message = if entry.name == lints::WARNINGS {
            format!(
                "`{}` = \"allow\" silences every warning, the preset's lints included",
                entry.name
            )
        } else if lints::default_level(&entry.name).is_none()
            && !lints::is_group(&entry.name)
            && lints::resolve(preset_entries, &entry.name).level.is_none()
        {
            format!(
                "`cargo bs` doesn't know the lint `{}`, so it can't tell whether allowing it loosens the preset; remove it, or use its current name",
                entry.name
            )
        } else {
            continue;
        };

        drifts.push(manifest.drift(
            Gate::Preset,
            line_of_entry(manifest, entry, header),
            message,
        ));
    }

    drifts
}

/// The lints the project leaves below the preset, by the entry that put them there (`None`:
/// nothing sets them, so they sit at their default).
fn turned_down<'table>(
    entries: &'table [Entry],
    preset_entries: &[Entry],
) -> BTreeMap<Option<&'table str>, Vec<Lowered>> {
    let mut names: Vec<&str> = lints::known().collect();

    names.extend(
        entries
            .iter()
            .chain(preset_entries)
            .map(|entry| entry.name.as_str()),
    );
    names.sort_unstable();
    names.dedup();

    let mut turned_down: BTreeMap<Option<&str>, Vec<Lowered>> = BTreeMap::new();

    for name in names.into_iter().filter(|name| !lints::is_group(name)) {
        let Some(wanted) = lints::resolve(preset_entries, name).level else {
            continue;
        };

        let have = lints::resolve(entries, name);

        if have.level.is_none_or(|level| level < wanted) {
            turned_down
                .entry(have.by.map(|entry| entry.name.as_str()))
                .or_default()
                .push(Lowered {
                    lint: name.to_owned(),
                    wanted,
                    have: have.level,
                });
        }
    }

    turned_down
}

fn lowered_message(entry: Option<&Entry>, lowered: &[Lowered]) -> String {
    let names = listed(lowered);

    let (Some(first), Some(entry)) = (lowered.first(), entry) else {
        return format!(
            "the lints table doesn't turn on {names} the way the preset does; `cargo bs init` prints the preset"
        );
    };

    let count = lowered.len();
    let level = entry.level.name();
    let have = shown(first.have);
    let wanted = first.wanted.name();

    if count > 1 {
        format!(
            "`{}` = \"{level}\" (priority {}) turns down {count} lints the preset turns on: {names}",
            entry.name, entry.priority
        )
    } else if entry.name == first.lint {
        format!(
            "`{}` = \"{have}\" turns down a lint the preset has at `{wanted}`",
            first.lint
        )
    } else {
        format!(
            "`{}` ends up `{have}` here, through `{}` = \"{level}\" (priority {}); the preset has it at `{wanted}`",
            first.lint, entry.name, entry.priority
        )
    }
}

fn line_of_entry(manifest: &Document, entry: &Entry, header: &str) -> usize {
    let key = entry.name.rsplit("::").next().unwrap_or(&entry.name);

    manifest.line_of(key, header)
}

fn shown(level: Option<Level>) -> &'static str {
    level.map_or("unknown", Level::name)
}

fn listed(lowered: &[Lowered]) -> String {
    let mut names: Vec<String> = lowered
        .iter()
        .take(NAMES_SHOWN)
        .map(|lowered| format!("`{}`", lowered.lint))
        .collect();

    if lowered.len() > NAMES_SHOWN {
        let hidden = lowered.len() - NAMES_SHOWN;

        names.push(format!("and {hidden} more"));
    }

    names.join(", ")
}

/// Members of the workspace (and the root package) that don't take `[lints] workspace = true`.
fn members_that_dont_inherit(root: &Path, manifest: &Document) -> Vec<Drift> {
    let patterns = |key: &str| -> Vec<String> {
        manifest
            .table_at(&["workspace"])
            .and_then(|workspace| workspace.get(key))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };

    let (members, excluded) = (patterns("members"), patterns("exclude"));

    files::named(root, &["Cargo.toml"])
        .into_iter()
        .filter(|path| {
            let directory = path
                .parent()
                .and_then(|parent| parent.strip_prefix(root).ok())
                .unwrap_or_else(|| Path::new(""));
            let is_member = |patterns: &[String]| patterns.iter().any(|pattern| files::glob_matches(pattern, directory));

            directory.as_os_str().is_empty() || (is_member(&members) && !is_member(&excluded))
        })
        .filter_map(|path| match Document::read(&path, Gate::Preset) {
            Err(drift) => Some(drift),
            Ok(member) => {
                let is_package = member.table.contains_key("package");
                let inherits = member
                    .table_at(&["lints"])
                    .and_then(|lints| lints.get("workspace"))
                    .and_then(Value::as_bool)
                    == Some(true);

                (is_package && !inherits).then(|| {
                    member.drift(Gate::Preset, member.line_of("workspace", "[lints]"), "this crate doesn't inherit the workspace lints; add `[lints] workspace = true`".to_owned())
                })
            }
        })
        .collect()
}

/// The clippy.toml clippy uses for the root (the nearest one at or above it) and every one
/// under it, each held to the preset's settings.
fn clippy_configs(root: &Path, preset: &Table) -> Vec<Drift> {
    let nearest = root.ancestors().find_map(|directory| {
        CLIPPY_FILES
            .iter()
            .map(|name| directory.join(name))
            .find(|path| path.is_file())
    });

    let Some(nearest) = nearest else {
        return vec![Drift {
            path: root.join("clippy.toml"),
            line: Some(1),
            gate: Gate::Preset,
            message: "there's no clippy.toml, so clippy runs without the preset's settings; run `cargo bs init`".to_owned(),
        }];
    };

    let mut configs = files::named(root, &CLIPPY_FILES);

    configs.push(nearest);
    configs.sort();
    configs.dedup();

    configs
        .iter()
        .flat_map(|path| match Document::read(path, Gate::Preset) {
            Err(drift) => vec![drift],
            Ok(config) => settings::loosened(preset, &config.table)
                .into_iter()
                .map(|found| {
                    let line = if found.set {
                        config.line_of(&found.key, "")
                    } else {
                        1
                    };

                    config.drift(Gate::Preset, line, found.message)
                })
                .collect(),
        })
        .collect()
}

/// `.cargo/config.toml` and `.cargo/config` in the root and every directory above it, which
/// Cargo all reads.
fn cargo_configs(root: &Path, preset_entries: &[Entry]) -> Vec<Drift> {
    let mut paths: Vec<PathBuf> = root
        .ancestors()
        .flat_map(|directory| CARGO_CONFIGS.iter().map(move |name| directory.join(name)))
        .collect();

    if let Some(home) = std::env::var_os("CARGO_HOME") {
        let home = PathBuf::from(home);

        paths.extend(["config.toml", "config"].iter().map(|name| home.join(name)));
    }

    paths.dedup();

    paths
        .iter()
        .filter(|path| path.is_file())
        .flat_map(|path| match Document::read(path, Gate::CargoConfig) {
            Err(drift) => vec![drift],
            Ok(config) => cargo_config::config_bypasses(&config.table, preset_entries)
                .into_iter()
                .map(|Bypass { key, message }| {
                    config.drift(Gate::CargoConfig, config.line_of(&key, ""), message)
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::check;
    use crate::preset::{CLIPPY, LINTS};

    /// A scratch project outside any git checkout, unique per run so parallel runs don't
    /// share it.
    fn project(case: &str, files: &[(&str, &str)]) -> PathBuf {
        let run = std::process::id();
        let directory = std::env::temp_dir().join(format!("cargo-bs-drift-{case}-{run}"));
        let _ignored = fs::remove_dir_all(&directory);

        for (name, text) in files {
            let path = directory.join(name);

            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        directory
    }

    fn drifts_in(case: &str, files: &[(&str, &str)]) -> Vec<String> {
        let directory = project(case, files);
        let messages = check(&directory)
            .into_iter()
            .filter(|drift| drift.path.starts_with(&directory))
            .map(|drift| drift.message)
            .collect();

        fs::remove_dir_all(&directory).unwrap();

        messages
    }

    fn manifest(lints: &str) -> String {
        format!("[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n\n[workspace]\n\n{lints}")
    }

    #[test]
    fn the_preset_itself_passes() {
        let found = drifts_in(
            "preset",
            &[("Cargo.toml", &manifest(LINTS)), ("clippy.toml", CLIPPY)],
        );

        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn a_lint_turned_down_is_reported() {
        let lints = LINTS.replace("unwrap_used = \"warn\"", "unwrap_used = \"allow\"");
        let found = drifts_in(
            "turned-down",
            &[("Cargo.toml", &manifest(&lints)), ("clippy.toml", CLIPPY)],
        );

        assert_eq!(found.len(), 1);
        assert!(found.iter().all(|message| message.contains("unwrap_used")));
    }

    #[test]
    fn allowing_a_lint_a_preset_group_turns_on_is_reported() {
        let lints = format!("{LINTS}too_many_lines = \"allow\"\nredundant_else = \"allow\"\n");
        let found = drifts_in(
            "group-member",
            &[("Cargo.toml", &manifest(&lints)), ("clippy.toml", CLIPPY)],
        );

        assert_eq!(found.len(), 2);
    }

    #[test]
    fn a_group_allowed_at_a_higher_priority_is_one_finding() {
        let lints = format!(
            "{LINTS}restriction = {{ level = \"allow\", priority = 1 }}\nstyle = \"allow\"\n"
        );
        let found = drifts_in(
            "priority",
            &[("Cargo.toml", &manifest(&lints)), ("clippy.toml", CLIPPY)],
        );

        assert_eq!(found.len(), 2);
        assert!(
            found
                .iter()
                .any(|message| message.contains("clippy::restriction"))
        );
    }

    #[test]
    fn stricter_levels_pass() {
        let lints = LINTS
            .replace("unsafe_code = \"deny\"", "unsafe_code = \"forbid\"")
            .replace(
                "unwrap_used = \"warn\"",
                "unwrap_used = \"deny\"\nnursery = { level = \"warn\", priority = -1 }",
            );
        let found = drifts_in(
            "stricter",
            &[("Cargo.toml", &manifest(&lints)), ("clippy.toml", CLIPPY)],
        );

        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn a_crate_that_drops_the_workspace_lints_is_reported() {
        let root = format!("[workspace]\nmembers = [\"crates/*\"]\n\n{LINTS}");
        let found = drifts_in(
            "no-inherit",
            &[
                ("Cargo.toml", &root),
                ("clippy.toml", CLIPPY),
                ("crates/core/Cargo.toml", "[package]\nname = \"core\"\n"),
                (
                    "crates/app/Cargo.toml",
                    "[package]\nname = \"app\"\n\n[lints]\nworkspace = true\n",
                ),
                ("fixtures/demo/Cargo.toml", "[package]\nname = \"demo\"\n"),
            ],
        );

        assert_eq!(found.len(), 1);
        assert!(found.iter().all(|message| message.contains("inherit")));
    }

    #[test]
    fn an_unparseable_manifest_is_reported() {
        let found = drifts_in(
            "unparseable",
            &[("Cargo.toml", "[package\n"), ("clippy.toml", CLIPPY)],
        );

        assert_eq!(found.len(), 1);
    }

    #[test]
    fn a_cargo_config_that_allows_lints_is_reported() {
        let found = drifts_in(
            "cargo-config",
            &[
                ("Cargo.toml", &manifest(LINTS)),
                ("clippy.toml", CLIPPY),
                (
                    ".cargo/config.toml",
                    "[build]\nrustflags = [\"-A\", \"clippy::unwrap_used\"]\n\n[env]\nCLIPPY_CONF_DIR = \"/nowhere\"\n",
                ),
            ],
        );

        assert_eq!(found.len(), 2);
    }
}
