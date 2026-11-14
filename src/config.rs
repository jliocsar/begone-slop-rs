//! `bs.toml`: the project's own boundaries, which no preset can know.
//!
//! ```toml
//! [io]
//! # Files and directories allowed to touch the outside world; everything else is pure.
//! allowed = ["src/main.rs", "src/shell"]
//! # Effects of your own, on top of the built-in list (fs, net, process, env, stdio, clock, ...).
//! effects = ["sqlx"]
//!
//! [[layers]]
//! crate = "crates/client"
//! # Top first. A module may import from the layers below its own, never above.
//! order = ["crate::conn", "crate::proto"]
//! ```
//!
//! A typo here would switch a boundary off without a word, so every entry is checked on load:
//! each `allowed` path and `crate` directory exists, each layer is a `crate::` path to a module
//! in that crate, and each effect is a path.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

const FILE_NAME: &str = "bs.toml";

/// Why `bs.toml` couldn't be used.
#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        cause: io::Error,
    },
    Parse {
        path: PathBuf,
        cause: toml::de::Error,
    },
    /// An entry that names nothing: a path or module that doesn't exist.
    Invalid {
        path: PathBuf,
        entry: String,
        problem: String,
    },
}

/// The parsed `bs.toml`, plus the directory it was found in. No file means no boundaries.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(skip)]
    pub root: PathBuf,
    pub io: Option<IoBoundary>,
    #[serde(default)]
    pub layers: Vec<Layers>,
}

/// `[io]`: where effects may happen.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IoBoundary {
    pub allowed: Vec<PathBuf>,
    #[serde(default)]
    pub effects: Vec<String>,
}

/// `[[layers]]`: one crate's module dependency direction, top layer first.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layers {
    #[serde(rename = "crate")]
    pub crate_root: PathBuf,
    pub order: Vec<ModulePath>,
}

/// A module written from its crate's root, `crate::conn::frame`, split into its segments. A
/// layer has to be one: `conn` alone would match no module, and the boundary would be off.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct ModulePath(Vec<String>);

/// A string that isn't a `crate::` module path.
#[derive(Debug, PartialEq, Eq)]
pub struct NotAModulePath(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, cause } => {
                write!(formatter, "can't read {}: {cause}", path.display())
            }
            Self::Parse { path, cause } => {
                write!(formatter, "{} isn't valid: {cause}", path.display())
            }
            Self::Invalid {
                path,
                entry,
                problem,
            } => write!(formatter, "{}: {entry}: {problem}", path.display()),
        }
    }
}

impl std::error::Error for ConfigError {}

impl ModulePath {
    /// The segments, `crate` first.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.0
    }
}

impl TryFrom<String> for ModulePath {
    type Error = NotAModulePath;

    fn try_from(written: String) -> Result<Self, Self::Error> {
        let segments: Vec<String> = written.split("::").map(str::to_owned).collect();
        let rooted = segments.first().is_some_and(|first| first == "crate");
        let named = segments
            .iter()
            .skip(1)
            .all(|segment| is_identifier(segment));

        if rooted && named {
            Ok(Self(segments))
        } else {
            Err(NotAModulePath(written))
        }
    }
}

impl fmt::Display for ModulePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.join("::"))
    }
}

impl fmt::Display for NotAModulePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "`{}` isn't a module path; write a layer from its crate's root, like `crate::conn`",
            self.0
        )
    }
}

impl std::error::Error for NotAModulePath {}

impl Config {
    /// Reads `bs.toml` from `root`, or returns the empty config when there is none.
    ///
    /// # Errors
    ///
    /// Fails when `root` can't be resolved, the file can't be read or parsed, or an entry
    /// names a path or module that doesn't exist.
    pub fn load(root: &Path) -> Result<Self, ConfigError> {
        let root = root.canonicalize().map_err(|cause| ConfigError::Read {
            path: root.to_owned(),
            cause,
        })?;
        let path = root.join(FILE_NAME);

        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    root,
                    ..Self::default()
                });
            }
            Err(cause) => return Err(ConfigError::Read { path, cause }),
        };

        let config = Self::parse(&text, root).map_err(|cause| ConfigError::Parse {
            path: path.clone(),
            cause,
        })?;

        config.problem().map_or(Ok(config), |(entry, problem)| {
            Err(ConfigError::Invalid {
                path,
                entry,
                problem,
            })
        })
    }

    /// Parses `bs.toml`'s text as the config of a project at `root`, without checking that
    /// its entries exist: that's [`Config::load`]'s job, which reads the disk.
    ///
    /// # Errors
    ///
    /// Fails when the text isn't a valid `bs.toml`.
    pub fn parse(text: &str, root: PathBuf) -> Result<Self, toml::de::Error> {
        let mut config: Self = toml::from_str(text)?;
        config.root = root;
        config.normalize();

        Ok(config)
    }

    /// `./src` and `src` mean the same directory; paths are compared component by component.
    fn normalize(&mut self) {
        let plain = |path: &Path| -> PathBuf {
            path.components()
                .filter(|component| *component != Component::CurDir)
                .collect()
        };

        if let Some(io) = &mut self.io {
            io.allowed = io.allowed.iter().map(|path| plain(path)).collect();
        }

        for layers in &mut self.layers {
            layers.crate_root = plain(&layers.crate_root);
        }
    }

    /// The first entry that names nothing, and why: `(entry, problem)`.
    fn problem(&self) -> Option<(String, String)> {
        let io = self.io.iter();
        let allowed = io.clone().flat_map(|io| &io.allowed).find_map(|allowed| {
            (!self.root.join(allowed).exists()).then(|| {
                (
                    format!("[io] allowed `{}`", allowed.display()),
                    "no such file or directory under the project's root".to_owned(),
                )
            })
        });
        let effects = io.flat_map(|io| &io.effects).find_map(|effect| {
            (!effect.split("::").all(is_identifier)).then(|| {
                (
                    format!("[io] effects `{effect}`"),
                    "an effect is a path like `sqlx` or `tokio::fs`".to_owned(),
                )
            })
        });

        allowed.or(effects).or_else(|| {
            self.layers
                .iter()
                .find_map(|layers| layers.problem(&self.root))
        })
    }
}

impl Layers {
    fn problem(&self, root: &Path) -> Option<(String, String)> {
        let directory = root.join(&self.crate_root);

        let written = if self.crate_root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &self.crate_root
        };

        let entry = format!("[[layers]] crate `{}`", written.display());

        if !directory.join("Cargo.toml").is_file() {
            return Some((entry, "no Cargo.toml there, so no crate".to_owned()));
        }

        self.order
            .iter()
            .find(|module| !module_exists(&directory.join("src"), module.segments()))
            .map(|module| {
                (
                    format!("{entry} order `{module}`"),
                    "no such module in the crate (no file or directory for it, and no inline `mod` in its parent)".to_owned(),
                )
            })
    }
}

/// Whether `segments` (`crate` first) names a module under `source` (a crate's `src`): a file
/// or directory of its own, or an inline `mod` in its parent's file.
fn module_exists(source: &Path, segments: &[String]) -> bool {
    let Some((last, parents)) = segments.split_last() else {
        return false;
    };

    if parents.is_empty() {
        return source.join("lib.rs").is_file() || source.join("main.rs").is_file();
    }

    let parent_directory: PathBuf = parents.iter().skip(1).collect();
    let own = source.join(&parent_directory).join(last);

    let parent_files = if parents.len() == 1 {
        vec![source.join("lib.rs"), source.join("main.rs")]
    } else {
        vec![
            source.join(&parent_directory).with_extension("rs"),
            source.join(&parent_directory).join("mod.rs"),
        ]
    };

    let inline = parent_files
        .iter()
        .any(|file| fs::read_to_string(file).is_ok_and(|text| declares_inline(&text, last)));

    own.with_extension("rs").is_file() || own.is_dir() || inline
}

/// Whether `text` opens an inline `mod name {`.
fn declares_inline(text: &str, name: &str) -> bool {
    let spaced = format!("mod {name} {{");
    let tight = format!("mod {name}{{");

    text.lines()
        .any(|line| line.contains(&spaced) || line.contains(&tight))
}

fn is_identifier(segment: &str) -> bool {
    let mut characters = segment.chars();

    characters
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && characters.all(|character| character.is_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Config, IoBoundary, Layers, ModulePath, NotAModulePath};

    fn module(written: &str) -> ModulePath {
        ModulePath::try_from(written.to_owned()).unwrap()
    }

    /// A config for this repo, which exists on disk, with `layers` and `allowed` as given.
    fn problem_of(allowed: &[&str], crate_root: &str, order: &[&str]) -> Option<String> {
        let config = Config {
            root: Path::new(env!("CARGO_MANIFEST_DIR")).to_owned(),
            io: Some(IoBoundary {
                allowed: allowed.iter().map(PathBuf::from).collect(),
                effects: vec!["sqlx".to_owned(), "tokio::fs".to_owned()],
            }),
            layers: vec![Layers {
                crate_root: crate_root.into(),
                order: order.iter().map(|written| module(written)).collect(),
            }],
        };

        config.problem().map(|(entry, _)| entry)
    }

    #[test]
    fn a_layer_is_a_crate_rooted_module_path() {
        assert_eq!(
            module("crate::conn::frame").segments(),
            ["crate", "conn", "frame"]
        );
        assert_eq!(module("crate::conn").to_string(), "crate::conn");
        assert_eq!(
            ModulePath::try_from("conn".to_owned()),
            Err(NotAModulePath("conn".to_owned()))
        );
        assert_eq!(
            ModulePath::try_from("crate::".to_owned()),
            Err(NotAModulePath("crate::".to_owned()))
        );
        assert_eq!(
            ModulePath::try_from("crate::a b".to_owned()),
            Err(NotAModulePath("crate::a b".to_owned()))
        );
    }

    #[test]
    fn existing_entries_pass() {
        let order = ["crate::check", "crate::rules::testing", "crate::source"];

        assert_eq!(
            problem_of(&["src/check.rs", "src/drift"], ".", &order),
            None
        );
    }

    #[test]
    fn an_entry_that_names_nothing_is_reported() {
        assert_eq!(
            problem_of(&["src/chek.rs"], ".", &["crate::check"]).as_deref(),
            Some("[io] allowed `src/chek.rs`")
        );
        assert_eq!(
            problem_of(&[], "crates/clinet", &["crate::check"]).as_deref(),
            Some("[[layers]] crate `crates/clinet`")
        );
        assert_eq!(
            problem_of(&[], ".", &["crate::check", "crate::rulez"]).as_deref(),
            Some("[[layers]] crate `.` order `crate::rulez`")
        );
    }

    #[test]
    fn a_layer_without_crate_fails_to_parse() {
        let parsed = toml::from_str::<Config>("[[layers]]\ncrate = \".\"\norder = [\"conn\"]\n");

        assert!(
            parsed
                .unwrap_err()
                .to_string()
                .contains("`conn` isn't a module path")
        );
    }
}
