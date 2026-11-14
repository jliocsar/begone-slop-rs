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
    pub order: Vec<String>,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, cause } => {
                write!(formatter, "can't read {}: {cause}", path.display())
            }
            Self::Parse { path, cause } => {
                write!(formatter, "{} isn't valid: {cause}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Reads `bs.toml` from `root`, or returns the empty config when there is none.
    ///
    /// # Errors
    ///
    /// Fails when `root` can't be resolved or the file can't be read or parsed.
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

        let mut config: Self =
            toml::from_str(&text).map_err(|cause| ConfigError::Parse { path, cause })?;
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

    /// `path` relative to the config's root, the form `bs.toml` paths are written in.
    #[must_use]
    pub fn relative(&self, path: &Path) -> Option<PathBuf> {
        let absolute = path.canonicalize().ok()?;

        absolute
            .strip_prefix(&self.root)
            .ok()
            .map(Path::to_path_buf)
    }
}
