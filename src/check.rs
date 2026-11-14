//! Runs the syntax rules over files, applies fixes until nothing fixable is left, and reports.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::module_paths::normalize;
use crate::rules::{self, Context, FileContext, Finding, Role};
use crate::source::Source;
use crate::{files, tree};

/// Each fix pass can expose another (a removed blank line changes a neighbour's gap), but
/// never more than a few deep.
const MAX_FIX_PASSES: usize = 8;

/// Whether a run only reports, or also rewrites what it can fix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Report,
    Fix,
}

/// Why a file couldn't be checked.
#[derive(Debug)]
pub enum CheckError {
    Read { path: PathBuf, cause: io::Error },
    Parse { path: PathBuf, cause: syn::Error },
    Write { path: PathBuf, cause: io::Error },
}

/// What a run knows about the project: its `bs.toml`, every Rust file it lists, and what kind
/// of code each file is.
pub struct Project {
    pub config: Config,
    base: PathBuf,
    listed: HashSet<PathBuf>,
    roles: HashMap<PathBuf, Role>,
}

/// One checked file: what's still wrong after fixing, and how many findings were fixed.
pub struct FileReport {
    pub source: Source,
    pub findings: Vec<Finding>,
    pub fixed: usize,
}

impl fmt::Display for CheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, cause } => {
                write!(formatter, "can't read {}: {cause}", path.display())
            }
            Self::Parse { path, cause } => {
                let line = cause.span().start().line;

                write!(
                    formatter,
                    "{}:{line}: unparsed-file: syn can't parse this file ({cause}), so no syntax rule checked it; rustc will say more",
                    path.display()
                )
            }
            Self::Write { path, cause } => {
                write!(formatter, "can't write {}: {cause}", path.display())
            }
        }
    }
}

impl std::error::Error for CheckError {}

impl Project {
    /// Lists the project's files and walks the module tree of the crates `files` belong to.
    ///
    /// # Errors
    ///
    /// Fails when the current directory or the project's files can't be read.
    pub fn new(config: Config, files: &[PathBuf]) -> io::Result<Self> {
        let base = std::env::current_dir()?;
        let listed = files::rust_files(std::slice::from_ref(&config.root))?
            .iter()
            .map(|path| normalize(&base.join(path)))
            .collect();
        let absolute: Vec<PathBuf> = files
            .iter()
            .map(|path| normalize(&base.join(path)))
            .collect();
        let roles = tree::roles(&absolute);

        Ok(Self {
            config,
            base,
            listed,
            roles,
        })
    }

    /// What the rules need to know about `path`: its role comes from the module tree, else
    /// from where it sits in its crate. Its path from the root is resolved here, once, so the
    /// rules never touch the disk.
    #[must_use]
    pub fn context(&self, path: &Path) -> Context<'_> {
        let absolute = normalize(&self.base.join(path));
        let role = self.roles.get(&absolute).copied().unwrap_or_else(|| {
            tree::crate_root(&absolute)
                .or(Some(self.config.root.as_path()))
                .and_then(|root| absolute.strip_prefix(root).ok())
                .map_or(Role::Library, Role::from_path)
        });

        let relative = path
            .canonicalize()
            .ok()
            .and_then(|real| Some(real.strip_prefix(&self.config.root).ok()?.to_path_buf()));

        Context {
            config: &self.config,
            role,
            absolute,
            relative,
            listed: &self.listed,
        }
    }
}

/// Checks one file, rewriting it first in [`Mode::Fix`].
///
/// # Errors
///
/// Fails when the file can't be read, parsed or written back.
pub fn check_file(path: &Path, mode: Mode, project: &Project) -> Result<FileReport, CheckError> {
    let text = fs::read_to_string(path).map_err(|cause| CheckError::Read {
        path: path.to_owned(),
        cause,
    })?;
    let mut source = Source::new(path.to_owned(), &text);
    let context = project.context(path);
    let mut findings = findings_in(&source, &context)?;
    let found = findings.len();

    if mode == Mode::Fix {
        for _ in 0..MAX_FIX_PASSES {
            let edits: Vec<_> = findings
                .iter()
                .filter_map(|finding| finding.edit.clone())
                .collect();

            if edits.is_empty() {
                break;
            }

            source.apply(&edits);
            findings = findings_in(&source, &context)?;
        }

        let fixed_text = source.text();

        if fixed_text != text {
            fs::write(path, fixed_text).map_err(|cause| CheckError::Write {
                path: path.to_owned(),
                cause,
            })?;
        }
    }

    let fixed = found.saturating_sub(findings.len());

    Ok(FileReport {
        source,
        findings,
        fixed,
    })
}

/// The syntax rules' findings for a source, without touching the disk.
///
/// # Errors
///
/// Fails when syn can't parse the source.
pub fn findings_in(source: &Source, context: &Context) -> Result<Vec<Finding>, CheckError> {
    let file = syn::parse_file(&source.text()).map_err(|cause| CheckError::Parse {
        path: source.path.clone(),
        cause,
    })?;
    let findings = rules::run(&FileContext::new(source, &file, context));
    proc_macro2::extra::invalidate_current_thread_spans();

    Ok(findings)
}
