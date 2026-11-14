//! `cargo bs`: begone-slop for Rust.

use std::fmt;
use std::path::PathBuf;
use std::process::ExitCode;

use cargo_bs::check::{self, CheckError, Mode};
use cargo_bs::config::Config;
use cargo_bs::{ast_grep, drift, files, init, preset};

const USAGE: &str = "\
usage: cargo bs [--fix] [--no-ast-grep] [PATH...]
       cargo bs init

Checks Rust files with the begone-slop rules: statement padding, assertion padding,
cyclomatic complexity and barrel modules, then the ast-grep pattern rules.
PATH defaults to the current directory.

  --fix          rewrite what can be fixed, report the rest
  --no-ast-grep  skip the pattern rules
  init           write the clippy.toml and rustfmt.toml presets and print the lints table
  -h, --help     show this";

enum Command {
    Check(Options),
    Help,
    Init,
}

struct Options {
    mode: Mode,
    ast_grep: bool,
    paths: Vec<PathBuf>,
}

#[derive(Debug)]
struct UnknownFlag(String);

impl fmt::Display for UnknownFlag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown flag `{}`\n\n{USAGE}", self.0)
    }
}

fn main() -> ExitCode {
    // Cargo runs `cargo-bs bs <args>`; run directly, there's no `bs`.
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();

    if arguments.first().is_some_and(|first| first == "bs") {
        arguments.remove(0);
    }

    match parse(arguments) {
        Ok(Command::Help) => {
            println!("{USAGE}");

            ExitCode::SUCCESS
        }
        Ok(Command::Init) => run_init(),
        Ok(Command::Check(options)) => run_check(&options),
        Err(error) => {
            eprintln!("cargo bs: {error}");

            ExitCode::from(2)
        }
    }
}

fn parse(arguments: Vec<String>) -> Result<Command, UnknownFlag> {
    if arguments.first().is_some_and(|first| first == "init") {
        return Ok(Command::Init);
    }

    let mut options = Options {
        mode: Mode::Report,
        ast_grep: true,
        paths: Vec::new(),
    };

    for argument in arguments {
        match argument.as_str() {
            "--fix" => options.mode = Mode::Fix,
            "--no-ast-grep" => options.ast_grep = false,
            "-h" | "--help" => return Ok(Command::Help),
            flag if flag.starts_with('-') => return Err(UnknownFlag(argument)),
            _ => options.paths.push(argument.into()),
        }
    }

    if options.paths.is_empty() {
        options.paths.push(".".into());
    }

    Ok(Command::Check(options))
}

fn run_init() -> ExitCode {
    match init::write_presets(".".as_ref()) {
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
            eprintln!("cargo bs init: {error}");

            ExitCode::FAILURE
        }
    }
}

fn run_check(options: &Options) -> ExitCode {
    let root = match std::env::current_dir() {
        Ok(directory) => files::project_root(&directory),
        Err(error) => {
            eprintln!("cargo bs: can't read the current directory: {error}");

            return ExitCode::from(2);
        }
    };

    let config = match Config::load(&root) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("cargo bs: {error}");

            return ExitCode::from(2);
        }
    };

    let paths = match files::rust_files(&options.paths) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("cargo bs: {error}");

            return ExitCode::from(2);
        }
    };

    let drifts = drift::check(&root);

    for found in &drifts {
        println!("{found}");
    }

    let mut problems = drifts.len();
    let mut fixed = 0;

    for path in &paths {
        match check::check_file(path, options.mode, &config) {
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
            Err(error @ CheckError::Parse { .. }) => eprintln!("warning: {error}"),
            Err(error) => {
                eprintln!("error: {error}");
                problems += 1;
            }
        }
    }

    let patterns_clean = !options.ast_grep || scan_patterns(options);

    if fixed > 0 {
        println!("cargo bs: fixed {fixed}");
    }

    if problems == 0 && patterns_clean {
        ExitCode::SUCCESS
    } else {
        println!(
            "cargo bs: {problems} syntax finding(s) left in {} file(s)",
            paths.len()
        );

        ExitCode::FAILURE
    }
}

fn scan_patterns(options: &Options) -> bool {
    match ast_grep::scan(&options.paths, options.mode) {
        Ok(clean) => clean,
        Err(error) => {
            eprintln!("error: {error}");

            false
        }
    }
}
