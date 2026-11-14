//! Runs every fixture under `tests/fixtures/<rule>/`.
//!
//! A fixture is a Rust file saved as `<case>.rs.fixture`, so the gate, which lints every `.rs`
//! file git lists, doesn't lint the slop it holds on purpose. Each line that should be flagged
//! ends with `//~ <rule>` (or `//~ <rule>, <rule>`). The markers are taken out before the rules
//! run, so they can't change what the rules see. The first line may be `//@ path: <path>`: where
//! the file pretends to live (`src/sample.rs` by default), which decides its role. A `bs.toml`
//! next to the fixtures is their config.
//!
//! The rules have to report exactly the marked lines, nothing more. And fixing a fixture has to
//! settle: the fixed text still parses, and fixing it again changes nothing.
//!
//! A fixture records what the rules report today. Many come from audit repros, so a few still
//! hold a loophole a rule misses (`loopholes.rs.fixture`); closing one means adding its marker.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use cargo_bs::check;
use cargo_bs::config::Config;
use cargo_bs::rules::{Context, Role};
use cargo_bs::source::Source;

const EXTENSION: &str = ".rs.fixture";
const MARKER: &str = "//~";
const PATH_DIRECTIVE: &str = "//@ path:";
const DEFAULT_PATH: &str = "src/sample.rs";
const PROJECT: &str = "/project";

/// One fixture, read: its code with the markers taken out, where it pretends to live, its
/// config, and the `(line, rule)` pairs its markers expect.
struct Fixture {
    file: PathBuf,
    path: String,
    config: Config,
    text: String,
    expected: Vec<(usize, String)>,
}

impl Fixture {
    fn read(file: PathBuf) -> io::Result<Self> {
        let raw = fs::read_to_string(&file)?;
        let lines: Vec<(&str, Option<&str>)> = raw
            .lines()
            .map(|line| match line.split_once(MARKER) {
                Some((code, rules)) => (code.trim_end(), Some(rules)),
                None => (line, None),
            })
            .collect();
        let mut expected: Vec<(usize, String)> = lines
            .iter()
            .zip(1..)
            .flat_map(|((_, rules), number)| {
                rules
                    .iter()
                    .flat_map(|rules| rules.split(','))
                    .map(str::trim)
                    .filter(|rule| !rule.is_empty())
                    .map(move |rule| (number, rule.to_owned()))
            })
            .collect();
        expected.sort();
        let path = lines
            .first()
            .and_then(|(code, _)| code.strip_prefix(PATH_DIRECTIVE))
            .map_or(DEFAULT_PATH, str::trim)
            .to_owned();

        let config = match fs::read_to_string(file.with_file_name("bs.toml")) {
            Ok(text) => Config::parse(&text, PROJECT.into()).map_err(io::Error::other)?,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => Config {
                root: PROJECT.into(),
                ..Config::default()
            },
            Err(cause) => return Err(cause),
        };

        let code: Vec<&str> = lines.iter().map(|(code, _)| *code).collect();

        Ok(Self {
            file,
            path,
            config,
            text: code.join("\n") + "\n",
            expected,
        })
    }

    fn context<'run>(&'run self, listed: &'run HashSet<PathBuf>) -> Context<'run> {
        Context {
            config: &self.config,
            role: Role::from_path(Path::new(&self.path)),
            absolute: Path::new(PROJECT).join(&self.path),
            relative: Some(PathBuf::from(&self.path)),
            listed,
        }
    }

    fn source(&self, text: &str) -> Source {
        Source::new(PathBuf::from(&self.path), text)
    }

    /// What the rules report, as `(line, rule)` pairs.
    fn found(&self) -> Result<Vec<(usize, String)>, check::CheckError> {
        let listed = HashSet::new();
        let mut found: Vec<(usize, String)> =
            check::findings_in(&self.source(&self.text), &self.context(&listed))?
                .into_iter()
                .map(|finding| (finding.line, finding.rule.name().to_owned()))
                .collect();
        found.sort();

        Ok(found)
    }

    /// `text` after `--fix`, or why the fixes didn't settle.
    fn fixed(&self, text: &str) -> Result<String, check::CheckError> {
        let listed = HashSet::new();
        let mut source = self.source(text);
        check::settle(&mut source, &self.context(&listed))?;

        Ok(source.text())
    }
}

/// Every fixture, in path order.
fn fixtures() -> io::Result<Vec<Fixture>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let directories = fs::read_dir(root)?
        .map(|entry| Ok(entry?.path()))
        .collect::<io::Result<Vec<PathBuf>>>()?;
    let mut files = directories
        .iter()
        .filter(|directory| directory.is_dir())
        .map(fs::read_dir)
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .map(|entry| Ok(entry?.path()))
        .collect::<io::Result<Vec<PathBuf>>>()?;
    files.retain(|file| file.to_string_lossy().ends_with(EXTENSION));
    files.sort();

    files.into_iter().map(Fixture::read).collect()
}

#[test]
fn every_fixture_reports_exactly_its_marked_lines() {
    let fixtures = fixtures().unwrap();
    let wrong: Vec<String> = fixtures
        .iter()
        .filter_map(|fixture| {
            let found = fixture.found().unwrap();

            (found != fixture.expected).then(|| {
                format!(
                    "{}\n  marked {:?}\n  found  {:?}",
                    fixture.file.display(),
                    fixture.expected,
                    found
                )
            })
        })
        .collect();

    assert!(!fixtures.is_empty(), "no fixtures under tests/fixtures");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn fixing_a_fixture_settles_and_fixing_again_changes_nothing() {
    let unsettled: Vec<String> = fixtures()
        .unwrap()
        .iter()
        .filter_map(|fixture| {
            let once = fixture.fixed(&fixture.text);
            let twice = once.as_ref().map(|text| fixture.fixed(text));

            match (&once, &twice) {
                (Ok(once), Ok(Ok(twice))) if once == twice => None,
                _ => Some(format!(
                    "{}\n  once  {once:?}\n  twice {twice:?}",
                    fixture.file.display()
                )),
            }
        })
        .collect();

    assert!(unsettled.is_empty(), "{}", unsettled.join("\n"));
}
