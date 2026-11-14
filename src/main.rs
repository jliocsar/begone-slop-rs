//! `cargo bs`: begone-slop for Rust.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cargo_bs::ast_grep::{self, AstGrepError};
use cargo_bs::check::{self, CheckError, Mode, Project};
use cargo_bs::config::{Config, ConfigError};
use cargo_bs::{drift, files, init, preset};

const USAGE: &str = "\
usage: cargo bs [--fix] [--no-ast-grep] [--] [PATH...]
       cargo bs init [DIR]

Checks Rust files with the begone-slop rules: statement padding, assertion padding,
cyclomatic complexity and barrel modules, then the ast-grep pattern rules.
PATH defaults to the current directory.

  --fix          rewrite what can be fixed, report the rest
  --no-ast-grep  skip the pattern rules
  --             read every argument after it as a path
  init           write the clippy.toml and rustfmt.toml presets and print the lints table
  -h, --help     show this
  -V, --version  print the version";

const INIT_USAGE: &str = "\
usage: cargo bs init [--] [DIR]

Writes clippy.toml and rustfmt.toml into DIR (the current directory by default), keeping any
that are already there, and prints the [workspace.lints] tables to paste into Cargo.toml.

  -h, --help     show this";

const HELP_FLAGS: [&str; 2] = ["-h", "--help"];

/// The `--` that ends the options: everything after it is a path.
const END_OF_OPTIONS: &str = "--";

enum Command {
    Check(Options),
    Help,
    Init(PathBuf),
    InitHelp,
    Version,
}

struct Options {
    mode: Mode,
    ast_grep: bool,
    paths: Vec<PathBuf>,
}

/// A command line `cargo bs` can't run, with the usage that says what it takes.
#[derive(Debug)]
struct Usage {
    problem: String,
    usage: &'static str,
}

/// Why a check couldn't run at all, which exits 2: there's no verdict to give.
#[derive(Debug)]
enum Fatal {
    CurrentDirectory(io::Error),
    Config(ConfigError),
    Listing(io::Error),
    Project(io::Error),
    AstGrep(AstGrepError),
}

impl fmt::Display for Usage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}\n\n{}", self.problem, self.usage)
    }
}

impl fmt::Display for Fatal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentDirectory(cause) => {
                write!(formatter, "can't read the current directory: {cause}")
            }
            Self::Config(cause) => write!(formatter, "{cause}"),
            Self::Listing(cause) => write!(formatter, "{cause}"),
            Self::Project(cause) => write!(formatter, "can't list the project's files: {cause}"),
            Self::AstGrep(cause) => write!(formatter, "{cause}"),
        }
    }
}

fn main() -> ExitCode {
    // Cargo runs `cargo-bs bs <args>`; run directly, there's no `bs`.
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();

    if arguments.first().is_some_and(|first| first == "bs") {
        arguments.remove(0);
    }

    match parse(&arguments) {
        Ok(Command::Help) => {
            println!("{USAGE}");

            ExitCode::SUCCESS
        }
        Ok(Command::InitHelp) => {
            println!("{INIT_USAGE}");

            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("cargo-bs {}", env!("CARGO_PKG_VERSION"));

            ExitCode::SUCCESS
        }
        Ok(Command::Init(directory)) => run_init(&directory),
        Ok(Command::Check(options)) => run_check(&options).unwrap_or_else(|fatal| {
            eprintln!("cargo bs: {fatal}");

            ExitCode::from(2)
        }),
        Err(usage) => {
            eprintln!("cargo bs: {usage}");

            ExitCode::from(2)
        }
    }
}

fn parse(arguments: &[String]) -> Result<Command, Usage> {
    if let Some(("init", rest)) = arguments
        .split_first()
        .map(|(first, rest)| (first.as_str(), rest))
    {
        return parse_init(rest);
    }

    let (flags, paths) = split_options(arguments);
    let mut options = Options {
        mode: Mode::Report,
        ast_grep: true,
        paths: Vec::new(),
    };

    for argument in flags {
        match argument.as_str() {
            "--fix" => options.mode = Mode::Fix,
            "--no-ast-grep" => options.ast_grep = false,
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            flag if flag.starts_with('-') => return Err(unknown_flag(flag, USAGE)),
            path => options.paths.push(path.into()),
        }
    }

    options.paths.extend(paths.iter().map(PathBuf::from));

    if options.paths.is_empty() {
        options.paths.push(".".into());
    }

    Ok(Command::Check(options))
}

/// `init`'s own arguments: help, or one directory at most. Anything else is a usage error, so
/// `init --help` and `init --typo` never write a file.
fn parse_init(arguments: &[String]) -> Result<Command, Usage> {
    let (flags, after) = split_options(arguments);

    if flags.iter().any(|flag| HELP_FLAGS.contains(&flag.as_str())) {
        return Ok(Command::InitHelp);
    }

    if let Some(flag) = flags.iter().find(|flag| flag.starts_with('-')) {
        return Err(unknown_flag(flag, INIT_USAGE));
    }

    match flags.iter().chain(after).collect::<Vec<_>>().as_slice() {
        [] => Ok(Command::Init(".".into())),
        [directory] => Ok(Command::Init(directory.into())),
        _ => Err(Usage {
            problem: "`init` takes one directory at most".to_owned(),
            usage: INIT_USAGE,
        }),
    }
}

/// The arguments before a `--`, and the ones after it, which are paths whatever they look like.
fn split_options(arguments: &[String]) -> (&[String], &[String]) {
    arguments
        .iter()
        .position(|argument| argument == END_OF_OPTIONS)
        .map_or((arguments, &[]), |at| {
            (
                arguments.get(..at).unwrap_or_default(),
                arguments.get(at + 1..).unwrap_or_default(),
            )
        })
}

fn unknown_flag(flag: &str, usage: &'static str) -> Usage {
    Usage {
        problem: format!("unknown flag `{flag}`"),
        usage,
    }
}

fn run_init(directory: &Path) -> ExitCode {
    match init::write_presets(directory) {
        Ok(outcomes) => {
            for outcome in outcomes {
                match outcome {
                    init::Outcome::Written(name) => println!("wrote {name}"),
                    init::Outcome::Kept(name) => println!("kept your {name}"),
                }
            }

            println!(
                "\nAdd these to the root Cargo.toml, and `[lints] workspace = true` to each crate:\n\n{}",
                preset::LINTS
            );

            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cargo bs init: {}: {error}", directory.display());

            ExitCode::FAILURE
        }
    }
}

/// Runs every check; the exit code is the verdict. A [`Fatal`] means no verdict was reached.
fn run_check(options: &Options) -> Result<ExitCode, Fatal> {
    let directory = std::env::current_dir().map_err(Fatal::CurrentDirectory)?;
    let root = files::project_root(&directory);
    let config = Config::load(&root).map_err(Fatal::Config)?;
    let (paths, mut problems) = listed_files(&options.paths).map_err(Fatal::Listing)?;
    let project = Project::new(config, &paths).map_err(Fatal::Project)?;
    let drifts = drift::check(&root);

    for found in &drifts {
        println!("{found}");
    }

    problems += drifts.len();
    let (found, fixed) = check_files(&paths, options.mode, &project);
    problems += found;

    let patterns = if options.ast_grep {
        ast_grep::scan(&paths, options.mode)
    } else {
        Ok(true)
    };

    if fixed > 0 {
        println!("cargo bs: fixed {fixed}");
    }

    let clean = patterns.map_err(Fatal::AstGrep)?;

    if clean && problems == 0 {
        return Ok(ExitCode::SUCCESS);
    }

    let patterns = if clean { "clean" } else { "failed" };

    println!(
        "cargo bs: {problems} finding(s) in {} file(s) and the gate's config; pattern rules {patterns}",
        paths.len()
    );

    Ok(ExitCode::FAILURE)
}

/// The files under each path, and how many paths had none: a path git ignores yields no
/// files, and a run that checked nothing there mustn't pass as clean.
fn listed_files(paths: &[PathBuf]) -> std::io::Result<(Vec<PathBuf>, usize)> {
    let mut found = Vec::new();
    let mut empty = 0;

    for path in paths {
        let listed = files::rust_files(std::slice::from_ref(path))?;

        if listed.is_empty() {
            println!(
                "{}: no Rust files here that git lists (ignored?), so nothing here was checked",
                path.display()
            );
            empty += 1;
        }

        found.extend(listed);
    }

    found.sort();
    found.dedup();

    Ok((found, empty))
}

/// Runs the syntax rules over every file; returns the findings left and the fixes made. A
/// file that can't be parsed is a finding: no rule could check it.
fn check_files(paths: &[PathBuf], mode: Mode, project: &Project) -> (usize, usize) {
    let mut problems = 0;
    let mut fixed = 0;

    for path in paths {
        match check::check_file(path, mode, project) {
            Ok(report) => {
                for finding in &report.findings {
                    println!(
                        "{}:{}: {}: {}",
                        report.source.path.display(),
                        finding.line,
                        finding.rule.name(),
                        finding.message
                    );
                }

                problems += report.findings.len();
                fixed += report.fixed;
            }
            Err(error @ CheckError::Parse { .. }) => {
                println!("{error}");
                problems += 1;
            }
            Err(error) => {
                eprintln!("error: {error}");
                problems += 1;
            }
        }
    }

    (problems, fixed)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Command, parse};

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    fn parsed(line: &str) -> Command {
        parse(&words(line)).unwrap()
    }

    fn paths_of(line: &str) -> Vec<PathBuf> {
        match parsed(line) {
            Command::Check(options) => options.paths,
            Command::Help | Command::Init(_) | Command::InitHelp | Command::Version => Vec::new(),
        }
    }

    fn init_directory(line: &str) -> Option<PathBuf> {
        match parsed(line) {
            Command::Init(directory) => Some(directory),
            Command::Check(_) | Command::Help | Command::InitHelp | Command::Version => None,
        }
    }

    #[test]
    fn init_help_writes_nothing() {
        assert!(matches!(parsed("init --help"), Command::InitHelp));
        assert!(matches!(parsed("init -h"), Command::InitHelp));
        assert!(matches!(parsed("init some/dir --help"), Command::InitHelp));
    }

    #[test]
    fn init_takes_one_directory() {
        assert_eq!(init_directory("init"), Some(".".into()));
        assert_eq!(init_directory("init some/dir"), Some("some/dir".into()));
        assert_eq!(init_directory("init -- -odd"), Some("-odd".into()));
    }

    #[test]
    fn init_rejects_what_it_doesnt_take() {
        let rejected = ["init --fix", "init one two"].map(|line| parse(&words(line)).is_err());

        assert_eq!(rejected, [true, true]);
    }

    #[test]
    fn the_version_flag_is_read() {
        assert!(matches!(parsed("-V"), Command::Version));
        assert!(matches!(parsed("--fix --version"), Command::Version));
    }

    #[test]
    fn a_double_dash_ends_the_options() {
        assert_eq!(
            paths_of("src -- --fix -V init"),
            [
                PathBuf::from("src"),
                "--fix".into(),
                "-V".into(),
                "init".into()
            ]
        );
        assert_eq!(paths_of("--fix"), [PathBuf::from(".")]);
    }
}
