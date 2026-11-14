//! The syntax rules. Each reads a parsed file and returns findings with line numbers.

mod abbreviations;
mod accumulator;
mod assert_padding;
mod async_blocking;
mod comments;
mod complexity;
mod exhaustive;
mod gap;
mod hidden_code;
mod io_boundary;
mod layers;
mod lint_waivers;
mod modules;
mod names;
mod padding;
mod paths;
mod prefer_match;
mod reexport_only;
mod signatures;
mod test_hygiene;
mod test_scope;
mod unit;
mod use_groups;

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::config::Config;
use crate::source::{Edit, Source};

/// One rule of the syntax engine. Its name is what `bs-allow(<name>)` waives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    AssertPadding,
    Complexity,
    IoBoundary,
    Layers,
    ModuleDoc,
    NewtypeIds,
    NoAbbreviations,
    NoBlockingInAsync,
    NoBooleanTrap,
    NoCommentedOutCode,
    NoCrateLevelAllow,
    NoForbiddenExpects,
    NoHiddenCode,
    NoJunkDrawerModules,
    NoMutableAccumulator,
    NoNumberedNames,
    NoSleepInTests,
    NoStringlyFields,
    NoTypeInName,
    NoWildcardOnLocalEnum,
    Padding,
    PreferMatch,
    ReexportOnly,
    TestMustAssert,
    TestName,
    UseGroups,
}

impl Rule {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::AssertPadding => "assert-padding",
            Self::Complexity => "complexity",
            Self::IoBoundary => "io-boundary",
            Self::Layers => "layers",
            Self::ModuleDoc => "module-doc",
            Self::NewtypeIds => "newtype-ids",
            Self::NoAbbreviations => "no-abbreviations",
            Self::NoBlockingInAsync => "no-blocking-in-async",
            Self::NoBooleanTrap => "no-boolean-trap",
            Self::NoCommentedOutCode => "no-commented-out-code",
            Self::NoCrateLevelAllow => "no-crate-level-allow",
            Self::NoForbiddenExpects => "no-forbidden-expects",
            Self::NoHiddenCode => "no-hidden-code",
            Self::NoJunkDrawerModules => "no-junk-drawer-modules",
            Self::NoMutableAccumulator => "no-mutable-accumulator",
            Self::NoNumberedNames => "no-numbered-names",
            Self::NoSleepInTests => "no-sleep-in-tests",
            Self::NoStringlyFields => "no-stringly-fields",
            Self::NoTypeInName => "no-type-in-name",
            Self::NoWildcardOnLocalEnum => "no-wildcard-on-local-enum",
            Self::Padding => "padding",
            Self::PreferMatch => "prefer-match",
            Self::ReexportOnly => "reexport-only-module",
            Self::TestMustAssert => "test-must-assert",
            Self::TestName => "test-name",
            Self::UseGroups => "use-groups",
        }
    }
}

impl Rule {
    /// Whether a `bs-allow` comment can waive the rule. The rules that guard the gate itself
    /// can't be waived: a waiver there is the bypass they exist to stop.
    #[must_use]
    pub const fn is_waivable(self) -> bool {
        !matches!(
            self,
            Self::NoCrateLevelAllow | Self::NoForbiddenExpects | Self::NoHiddenCode
        )
    }
}

/// What kind of code a file is, which decides the rules that apply to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Bench,
    Binary,
    BuildScript,
    Example,
    Library,
    Test,
}

/// What the run knows about a file beyond its text.
pub struct Context<'run> {
    pub config: &'run Config,
    pub role: Role,
    /// The file's absolute path, which `#[path]` and `include!` resolve against.
    pub absolute: PathBuf,
    /// Every Rust file the project lists (by absolute path): the code something checks.
    pub listed: &'run HashSet<PathBuf>,
}

impl Role {
    /// The role Cargo's layout gives a path relative to its crate's root: `tests/` and the
    /// like only directly under it.
    #[must_use]
    pub fn from_path(relative: &Path) -> Self {
        let parts: Vec<&str> = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(part) => part.to_str(),
                _ => None,
            })
            .collect();

        match parts.as_slice() {
            ["build.rs"] => Self::BuildScript,
            ["tests", ..] => Self::Test,
            ["examples", ..] => Self::Example,
            ["benches", ..] => Self::Bench,
            ["src", "main.rs"] | ["src", "bin", ..] => Self::Binary,
            _ => Self::Library,
        }
    }

    #[must_use]
    pub fn is_test(self) -> bool {
        self == Self::Test
    }

    /// Of two roles, the one more rules apply to: a library is held to the most.
    #[must_use]
    pub fn stricter(self, other: Self) -> Self {
        if other.rank() < self.rank() {
            other
        } else {
            self
        }
    }

    #[must_use]
    pub fn is_at_least_as_strict_as(self, other: Self) -> bool {
        self.rank() <= other.rank()
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Library => 0,
            Self::Binary => 1,
            Self::BuildScript => 2,
            Self::Example => 3,
            Self::Bench => 4,
            Self::Test => 5,
        }
    }
}

impl Context<'_> {
    #[must_use]
    pub fn in_tests(&self) -> bool {
        self.role.is_test()
    }
}

/// What a rule found, where, and the edit that fixes it when one exists.
#[derive(Clone, Debug)]
pub struct Finding {
    pub line: usize,
    pub rule: Rule,
    pub message: String,
    pub edit: Option<Edit>,
}

/// Runs every syntax rule on one file, minus the findings a `bs-allow` comment waives.
#[must_use]
pub fn run(source: &Source, file: &syn::File, context: &Context) -> Vec<Finding> {
    let mut findings = padding::check(source, file);
    findings.extend(assert_padding::check(source, file, context));
    findings.extend(complexity::check(file));
    findings.extend(reexport_only::check(source, file));
    findings.extend(abbreviations::check(file));
    findings.extend(test_hygiene::check(file, context));
    findings.extend(use_groups::check(source, file));
    findings.extend(lint_waivers::check(file, context));
    findings.extend(accumulator::check(file));
    findings.extend(prefer_match::check(file));
    findings.extend(signatures::check(file, context));
    findings.extend(io_boundary::check(source, file, context));
    findings.extend(layers::check(source, file, context));
    findings.extend(names::check(file));
    findings.extend(modules::check(source, file));
    findings.extend(comments::check(source));
    findings.extend(exhaustive::check(file, context));
    findings.extend(async_blocking::check(file));
    findings.extend(hidden_code::check(file, context));
    findings.retain(|finding| {
        !finding.rule.is_waivable() || !source.waives(finding.line, finding.rule.name())
    });
    findings.sort_by_key(|finding| finding.line);

    findings
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::HashSet;
    use std::path::Path;

    use super::{Context, Finding, Role, Rule};
    use crate::config::Config;
    use crate::source::Source;

    pub(crate) const NONE: [usize; 0] = [];

    /// The findings for `text` as the file at `path` in a project at `/project`, where the
    /// project lists the files in `listed`.
    pub(crate) fn findings_listing(path: &str, text: &str, listed: &[&str]) -> Vec<Finding> {
        let source = Source::new(path.into(), text);
        let file = syn::parse_file(text).expect("test input parses");
        let config = Config::default();
        let listed: HashSet<_> = listed
            .iter()
            .map(|name| Path::new("/project").join(name))
            .collect();
        let context = Context {
            config: &config,
            role: Role::from_path(Path::new(path)),
            absolute: Path::new("/project").join(path),
            listed: &listed,
        };

        super::run(&source, &file, &context)
    }

    pub(crate) fn findings(path: &str, text: &str) -> Vec<Finding> {
        findings_listing(path, text, &[])
    }

    pub(crate) fn lines_of(rule: Rule, text: &str) -> Vec<usize> {
        lines_in(rule, "src/sample.rs", text)
    }

    pub(crate) fn lines_in(rule: Rule, path: &str, text: &str) -> Vec<usize> {
        findings(path, text)
            .into_iter()
            .filter(|finding| finding.rule == rule)
            .map(|finding| finding.line)
            .collect()
    }

    pub(crate) fn fixed(text: &str) -> String {
        let mut source = Source::new("src/sample.rs".into(), text);
        let edits: Vec<_> = findings("src/sample.rs", text)
            .into_iter()
            .filter_map(|finding| finding.edit)
            .collect();
        source.apply(&edits);

        source.text()
    }
}
