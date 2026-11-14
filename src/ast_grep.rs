//! The pattern rules, run through the `ast-grep` CLI with the rule files this binary carries.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::Command;

use crate::check::Mode;

/// The files in `rules/`, by id. A test keeps this list and the directory equal.
pub const RULES: [(&str, &str); 15] = [
    (
        "error-message-style",
        include_str!("../rules/error-message-style.yml"),
    ),
    (
        "ignore-needs-reason",
        include_str!("../rules/ignore-needs-reason.yml"),
    ),
    (
        "no-box-dyn-error",
        include_str!("../rules/no-box-dyn-error.yml"),
    ),
    (
        "no-catch-unwind",
        include_str!("../rules/no-catch-unwind.yml"),
    ),
    (
        "no-detached-tasks",
        include_str!("../rules/no-detached-tasks.yml"),
    ),
    (
        "no-glob-reexport",
        include_str!("../rules/no-glob-reexport.yml"),
    ),
    (
        "no-global-state",
        include_str!("../rules/no-global-state.yml"),
    ),
    ("no-rc-refcell", include_str!("../rules/no-rc-refcell.yml")),
    (
        "no-string-error",
        include_str!("../rules/no-string-error.yml"),
    ),
    (
        "no-stringified-error",
        include_str!("../rules/no-stringified-error.yml"),
    ),
    (
        "no-stringly-match",
        include_str!("../rules/no-stringly-match.yml"),
    ),
    (
        "no-swallowed-parse",
        include_str!("../rules/no-swallowed-parse.yml"),
    ),
    (
        "no-untyped-json",
        include_str!("../rules/no-untyped-json.yml"),
    ),
    (
        "todo-needs-issue",
        include_str!("../rules/todo-needs-issue.yml"),
    ),
    (
        "waiver-needs-reason",
        include_str!("../rules/waiver-needs-reason.yml"),
    ),
];

/// A config file with nothing in it, so a project's `sgconfig.yml` isn't read.
#[cfg(not(windows))]
const EMPTY_CONFIG: &str = "/dev/null";

#[cfg(windows)]
const EMPTY_CONFIG: &str = "NUL";

/// Every kind of ignore file ast-grep would otherwise honour.
const NO_IGNORE: [&str; 6] = ["dot", "exclude", "global", "hidden", "parent", "vcs"];

/// Why the pattern rules didn't run.
#[derive(Debug)]
pub enum AstGrepError {
    Missing,
    Spawn(io::Error),
}

impl fmt::Display for AstGrepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str(
                "ast-grep isn't on PATH, so the pattern rules didn't run; install it (`cargo install ast-grep` or `mise use ast-grep`), or pass --no-ast-grep",
            ),
            Self::Spawn(cause) => write!(formatter, "couldn't run ast-grep: {cause}"),
        }
    }
}

impl std::error::Error for AstGrepError {}

/// Scans exactly `files` with every pattern rule; ast-grep prints its own report. Returns
/// whether the scan came back clean.
///
/// The files are the ones git lists, passed one by one, with ast-grep's own ignore handling
/// off and an empty config: a project's `.ignore` or `sgconfig.yml` (`languageGlobs` can
/// remap `.rs`) mustn't hide files from these rules.
///
/// # Errors
///
/// Fails when `ast-grep` is missing or can't be started.
pub fn scan(files: &[PathBuf], mode: Mode) -> Result<bool, AstGrepError> {
    if files.is_empty() {
        return Ok(true);
    }

    let rules: Vec<&str> = RULES.iter().map(|(_, text)| *text).collect();
    let mut command = Command::new("ast-grep");
    command
        .args(["scan", "--config", EMPTY_CONFIG, "--inline-rules"])
        .arg(rules.join("\n---\n"));

    for ignore in NO_IGNORE {
        command.args(["--no-ignore", ignore]);
    }

    if mode == Mode::Fix {
        command.arg("--update-all");
    }

    let status = command
        .arg("--")
        .args(files)
        .status()
        .map_err(|cause| match cause.kind() {
            io::ErrorKind::NotFound => AstGrepError::Missing,
            _ => AstGrepError::Spawn(cause),
        })?;

    Ok(status.success())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::RULES;

    #[test]
    fn every_rule_file_is_embedded() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("rules");
        let mut on_disk: Vec<String> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter_map(|path| Some(path.file_stem()?.to_str()?.to_owned()))
            .collect();
        on_disk.sort();
        let embedded: Vec<&str> = RULES.iter().map(|(id, _)| *id).collect();

        assert_eq!(on_disk, embedded);
    }

    #[test]
    fn each_rule_declares_its_own_id() {
        for (id, text) in RULES {
            assert!(text.starts_with(&format!("id: {id}\n")), "{id}");
        }
    }
}
