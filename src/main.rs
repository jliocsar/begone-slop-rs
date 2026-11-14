//! `cargo bs`: begone-slop for Rust.

use std::fmt;
use std::path::PathBuf;
use std::process::ExitCode;

use cargo_bs::check::{self, CheckError};
use cargo_bs::{ast_grep, files, init};

const USAGE: &str = "\
usage: cargo bs [--fix] [--no-ast-grep] [PATH...]
       cargo bs init

Checks Rust files with the begone-slop rules: statement padding, assertion padding,
cyclomatic complexity, file length and barrel modules, then the ast-grep pattern rules.
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
    fix: bool,
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
    let arguments: Vec<String> = std::env::args()
        .skip(1)
        .skip_while(|argument| argument == "bs")
        .collect();

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
        fix: false,
        ast_grep: true,
        paths: Vec::new(),
    };

    for argument in arguments {
        match argument.as_str() {
            "--fix" => options.fix = true,
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
                init::LINTS
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
    let paths = match files::rust_files(&options.paths) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("cargo bs: {error}");

            return ExitCode::from(2);
        }
    };

    let mut problems = 0;
    let mut fixed = 0;

    for path in &paths {
        match check::check_file(path, options.fix) {
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
    match ast_grep::scan(&options.paths, options.fix) {
        Ok(clean) => clean,
        Err(error) => {
            eprintln!("error: {error}");

            false
        }
    }
}
