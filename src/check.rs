//! Runs the syntax rules over files, applies fixes until nothing fixable is left, and reports.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::rules::{self, Finding};
use crate::source::Source;

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
                    "{}:{line}: skipped, syn can't parse it ({cause}); rustc will say more",
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

/// Checks one file, rewriting it first in [`Mode::Fix`].
///
/// # Errors
///
/// Fails when the file can't be read, parsed or written back.
pub fn check_file(path: &Path, mode: Mode, config: &Config) -> Result<FileReport, CheckError> {
    let text = fs::read_to_string(path).map_err(|cause| CheckError::Read {
        path: path.to_owned(),
        cause,
    })?;
    let mut source = Source::new(path.to_owned(), &text);
    let mut findings = findings_in(&source, config)?;
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
            findings = findings_in(&source, config)?;
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
pub fn findings_in(source: &Source, config: &Config) -> Result<Vec<Finding>, CheckError> {
    let file = syn::parse_file(&source.text()).map_err(|cause| CheckError::Parse {
        path: source.path.clone(),
        cause,
    })?;
    let findings = rules::run(source, &file, config);
    proc_macro2::extra::invalidate_current_thread_spans();

    Ok(findings)
}
