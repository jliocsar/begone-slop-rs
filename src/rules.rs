//! The syntax rules. Each reads a parsed file and returns findings with line numbers.

mod abbreviations;
mod accumulator;
mod assert_padding;
mod async_blocking;
mod comments;
mod complexity;
mod exhaustive;
mod file_context;
mod gap;
mod hidden_code;
mod io_boundary;
mod layers;
mod library_errors;
mod lint_waivers;
mod modules;
mod names;
mod padding;
mod panic_bypass;
mod paths;
mod prefer_match;
mod reexport_only;
mod signatures;
mod silent_errors;
mod test_honesty;
mod test_hygiene;
mod test_scope;
mod unit;
mod use_groups;
mod waivers;

use std::collections::HashSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

pub use file_context::FileContext;

use crate::config::Config;
use crate::source::Edit;

/// Declares every syntax rule once: the [`Rule`] enum, the [`RULES`] table and
/// [`Rule::info`] all come from this one list, so they can't disagree.
macro_rules! rules {
    ($($variant:ident => $name:literal, $group:ident, $fixable:literal;)*) => {
        /// Every syntax rule, in the order of [`Rule`].
        pub const RULES: &[RuleInfo] = &[
            $(RuleInfo { rule: Rule::$variant, name: $name, group: Group::$group, fixable: $fixable },)*
        ];

        /// One rule of the syntax engine. Its name is what `bs-allow(<name>)` waives.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Rule {
            $($variant,)*
        }

        impl Rule {
            /// The rule's row in [`RULES`].
            #[must_use]
            pub const fn info(self) -> RuleInfo {
                match self {
                    $(Self::$variant => RuleInfo { rule: self, name: $name, group: Group::$group, fixable: $fixable },)*
                }
            }
        }
    };
}

/// Every check the syntax engine runs, in order. Each reads one file and returns what it
/// found, for one rule or several.
const CHECKS: [fn(&FileContext) -> Vec<Finding>; 24] = [
    padding::check,
    assert_padding::check,
    complexity::check,
    reexport_only::check,
    abbreviations::check,
    test_hygiene::check,
    use_groups::check,
    lint_waivers::check,
    accumulator::check,
    prefer_match::check,
    signatures::check,
    io_boundary::check,
    layers::check,
    names::check,
    modules::check,
    comments::check,
    exhaustive::check,
    async_blocking::check,
    hidden_code::check,
    waivers::check,
    silent_errors::check,
    panic_bypass::check,
    library_errors::check,
    test_honesty::check,
];

rules! {
    AssertPadding => "assert-padding", Syntax, true;
    Complexity => "complexity", Syntax, false;
    IoBoundary => "io-boundary", Boundary, false;
    Layers => "layers", Boundary, false;
    ModuleDoc => "module-doc", Syntax, false;
    NewtypeIds => "newtype-ids", Syntax, false;
    NoAbbreviations => "no-abbreviations", Syntax, false;
    NoAnyhowInLib => "no-anyhow-in-lib", Syntax, false;
    NoBlockingInAsync => "no-blocking-in-async", Syntax, false;
    NoBooleanTrap => "no-boolean-trap", Syntax, false;
    NoCommentedOutCode => "no-commented-out-code", Syntax, false;
    NoCrateLevelAllow => "no-crate-level-allow", Gate, false;
    NoDiscardAssign => "no-discard-assign", Syntax, false;
    NoFilterMapOk => "no-filter-map-ok", Syntax, false;
    NoForbiddenExpects => "no-forbidden-expects", Gate, false;
    NoHiddenCode => "no-hidden-code", Gate, false;
    NoJunkDrawerModules => "no-junk-drawer-modules", Syntax, false;
    NoMutableAccumulator => "no-mutable-accumulator", Syntax, false;
    NoNumberedNames => "no-numbered-names", Syntax, false;
    NoPanicBypass => "no-panic-bypass", Syntax, false;
    NoSilentErrorArm => "no-silent-err-arm", Syntax, false;
    NoSleepInTests => "no-sleep-in-tests", Syntax, false;
    NoStringlyFields => "no-stringly-fields", Syntax, false;
    NoTestSwitchesInProduction => "no-cfg-test-in-prod", Syntax, false;
    NoTypeInName => "no-type-in-name", Syntax, false;
    NoWildcardOnLocalEnum => "no-wildcard-on-local-enum", Syntax, false;
    Padding => "padding", Syntax, true;
    PreferMatch => "prefer-match", Syntax, false;
    ReexportOnly => "reexport-only-module", Syntax, false;
    TestConditionalAssert => "test-conditional-assert", Syntax, false;
    TestEarlyReturn => "test-early-return", Syntax, false;
    TestMustAssert => "test-must-assert", Syntax, false;
    TestName => "test-name", Syntax, false;
    UseGroups => "use-groups", Syntax, true;
    WaiverNeedsReason => "waiver-needs-reason", Gate, false;
}

/// Where a rule belongs, which decides whether a waiver can reach it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Guards the gate itself: a waiver there is the bypass the rule exists to stop.
    Gate,
    /// Reads the boundaries `bs.toml` declares.
    Boundary,
    /// Every other syntax rule.
    Syntax,
}

/// What the table says about one rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleInfo {
    pub rule: Rule,
    /// What `bs-allow(<name>)` and the report call it.
    pub name: &'static str,
    pub group: Group,
    /// Whether `--fix` rewrites what it finds.
    pub fixable: bool,
}

/// A rule name no rule has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownRule(pub String);

impl Rule {
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.info().name
    }

    /// The rule called `name`, the way `bs-allow(<name>)` writes it.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        name.parse().ok()
    }

    /// Whether a `bs-allow` comment can waive the rule. The rules that guard the gate itself
    /// can't be waived: a waiver there is the bypass they exist to stop.
    #[must_use]
    pub const fn is_waivable(self) -> bool {
        !matches!(self.info().group, Group::Gate)
    }
}

impl FromStr for Rule {
    type Err = UnknownRule;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        RULES
            .iter()
            .find(|info| info.name == name)
            .map(|info| info.rule)
            .ok_or_else(|| UnknownRule(name.to_owned()))
    }
}

impl fmt::Display for UnknownRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "no rule is called `{}`", self.0)
    }
}

impl std::error::Error for UnknownRule {}

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
    /// The file's path from the project's root, the form `bs.toml` paths are written in;
    /// none when the file sits outside the root.
    pub relative: Option<PathBuf>,
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
pub fn run(file: &FileContext) -> Vec<Finding> {
    let mut findings: Vec<Finding> = CHECKS.iter().flat_map(|check| check(file)).collect();
    findings.retain(|finding| {
        !finding.rule.is_waivable() || !file.source.waives(finding.line, finding.rule.name())
    });
    findings.sort_by_key(|finding| finding.line);

    findings
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::HashSet;
    use std::path::Path;

    use super::{Context, FileContext, Finding, Role, Rule};
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
            relative: Some(path.into()),
            listed: &listed,
        };

        super::run(&FileContext::new(&source, &file, &context))
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

#[cfg(test)]
mod tests {
    use super::{Group, RULES, Rule, UnknownRule};

    #[test]
    fn a_rule_name_parses_back_to_its_rule() {
        for info in RULES {
            assert_eq!(info.name.parse::<Rule>(), Ok(info.rule));
            assert_eq!(info.rule.info(), *info);
        }

        assert_eq!(
            "complexty".parse::<Rule>(),
            Err(UnknownRule("complexty".to_owned()))
        );
    }

    #[test]
    fn only_the_gate_rules_are_unwaivable() {
        let unwaivable: Vec<&str> = RULES
            .iter()
            .filter(|info| !info.rule.is_waivable())
            .map(|info| info.name)
            .collect();

        assert_eq!(
            unwaivable,
            [
                "no-crate-level-allow",
                "no-forbidden-expects",
                "no-hidden-code",
                "waiver-needs-reason"
            ]
        );
        assert!(
            RULES
                .iter()
                .all(|info| info.rule.is_waivable() == (info.group != Group::Gate))
        );
    }

    /// The README documents every rule: each syntax rule in its table, with the fix column
    /// right, and each gate and boundary rule in its own bullet.
    #[test]
    fn the_readme_lists_every_rule() {
        let readme = include_str!("../README.md");
        let section = readme
            .split("## Syntax rules")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .unwrap();
        let mut table: Vec<(&str, bool)> = section
            .lines()
            .filter_map(|line| line.strip_prefix("| `")?.split_once('`'))
            .map(|(name, rest)| (name, rest.contains('✅')))
            .collect();
        let mut syntax: Vec<(&str, bool)> = RULES
            .iter()
            .filter(|info| info.group == Group::Syntax)
            .map(|info| (info.name, info.fixable))
            .collect();
        table.sort_unstable();
        syntax.sort_unstable();
        let unlisted: Vec<&str> = RULES
            .iter()
            .filter(|info| info.group != Group::Syntax)
            .map(|info| info.name)
            .filter(|name| !readme.contains(&format!("- `{name}`:")))
            .collect();

        assert_eq!(table, syntax);
        assert_eq!(unlisted, Vec::<&str>::new());
    }
}
