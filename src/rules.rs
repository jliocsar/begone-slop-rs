//! The syntax rules. Each reads a parsed file and returns findings with line numbers.

mod abbreviations;
mod accumulator;
mod assert_padding;
mod complexity;
mod gap;
mod io_boundary;
mod layers;
mod lint_waivers;
mod padding;
mod paths;
mod prefer_match;
mod reexport_only;
mod signatures;
mod test_hygiene;
mod test_scope;
mod unit;
mod use_groups;

use crate::config::Config;
use crate::source::{Edit, Source};

/// One rule of the syntax engine. Its name is what `bs-allow(<name>)` waives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    AssertPadding,
    Complexity,
    IoBoundary,
    Layers,
    NewtypeIds,
    NoAbbreviations,
    NoBooleanTrap,
    NoCrateLevelAllow,
    NoForbiddenExpects,
    NoMutableAccumulator,
    NoSleepInTests,
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
            Self::NewtypeIds => "newtype-ids",
            Self::NoAbbreviations => "no-abbreviations",
            Self::NoBooleanTrap => "no-boolean-trap",
            Self::NoCrateLevelAllow => "no-crate-level-allow",
            Self::NoForbiddenExpects => "no-forbidden-expects",
            Self::NoMutableAccumulator => "no-mutable-accumulator",
            Self::NoSleepInTests => "no-sleep-in-tests",
            Self::Padding => "padding",
            Self::PreferMatch => "prefer-match",
            Self::ReexportOnly => "reexport-only-module",
            Self::TestMustAssert => "test-must-assert",
            Self::TestName => "test-name",
            Self::UseGroups => "use-groups",
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
pub fn run(source: &Source, file: &syn::File, config: &Config) -> Vec<Finding> {
    let mut findings = padding::check(source, file);
    findings.extend(assert_padding::check(source, file));
    findings.extend(complexity::check(file));
    findings.extend(reexport_only::check(source, file));
    findings.extend(abbreviations::check(file));
    findings.extend(test_hygiene::check(source, file));
    findings.extend(use_groups::check(source, file));
    findings.extend(lint_waivers::check(source, file));
    findings.extend(accumulator::check(file));
    findings.extend(prefer_match::check(file));
    findings.extend(signatures::check(source, file));
    findings.extend(io_boundary::check(source, file, config));
    findings.extend(layers::check(source, file, config));
    findings.retain(|finding| !source.waives(finding.line, finding.rule.name()));
    findings.sort_by_key(|finding| finding.line);

    findings
}

#[cfg(test)]
pub(crate) mod testing {
    use super::{Finding, Rule};
    use crate::config::Config;
    use crate::source::Source;

    pub(crate) const NONE: [usize; 0] = [];

    pub(crate) fn findings(path: &str, text: &str) -> Vec<Finding> {
        let source = Source::new(path.into(), text);
        let file = syn::parse_file(text).expect("test input parses");

        super::run(&source, &file, &Config::default())
    }

    pub(crate) fn lines_of(rule: Rule, text: &str) -> Vec<usize> {
        findings("src/sample.rs", text)
            .into_iter()
            .filter(|finding| finding.rule == rule)
            .map(|finding| finding.line)
            .collect()
    }

    pub(crate) fn fixed(text: &str) -> String {
        let mut source = Source::new("src/sample.rs".into(), text);
        let file = syn::parse_file(text).expect("test input parses");
        let edits: Vec<_> = super::run(&source, &file, &Config::default())
            .into_iter()
            .filter_map(|finding| finding.edit)
            .collect();
        source.apply(&edits);

        source.text()
    }
}
