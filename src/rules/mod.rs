//! The syntax rules. Each reads a parsed file and returns findings with line numbers.

mod assert_padding;
mod complexity;
mod gap;
mod max_lines;
mod padding;
mod reexport_only;
mod unit;

use crate::source::{Edit, Source};

/// One rule of the syntax engine. Its name is what `bs-allow(<name>)` waives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    AssertPadding,
    Complexity,
    MaxLines,
    Padding,
    ReexportOnly,
}

impl Rule {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::AssertPadding => "assert-padding",
            Self::Complexity => "complexity",
            Self::MaxLines => "max-lines",
            Self::Padding => "padding",
            Self::ReexportOnly => "reexport-only-module",
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
pub fn run(source: &Source, file: &syn::File) -> Vec<Finding> {
    let mut findings = padding::check(source, file);
    findings.extend(assert_padding::check(source, file));
    findings.extend(complexity::check(file));
    findings.extend(max_lines::check(source));
    findings.extend(reexport_only::check(source, file));
    findings.retain(|finding| !source.waives(finding.line, finding.rule.name()));
    findings.sort_by_key(|finding| finding.line);

    findings
}

#[cfg(test)]
pub(crate) mod testing {
    use super::{Finding, Rule};
    use crate::source::Source;

    pub const NONE: [usize; 0] = [];

    pub fn findings(path: &str, text: &str) -> Vec<Finding> {
        let source = Source::new(path.into(), text);
        let file = syn::parse_file(text).expect("test input parses");

        super::run(&source, &file)
    }

    pub fn lines_of(rule: Rule, text: &str) -> Vec<usize> {
        findings("src/sample.rs", text)
            .into_iter()
            .filter(|finding| finding.rule == rule)
            .map(|finding| finding.line)
            .collect()
    }

    pub fn fixed(text: &str) -> String {
        let mut source = Source::new("src/sample.rs".into(), text);
        let file = syn::parse_file(text).expect("test input parses");
        let edits: Vec<_> = super::run(&source, &file)
            .into_iter()
            .filter_map(|finding| finding.edit)
            .collect();
        source.apply(&edits);

        source.text()
    }
}
