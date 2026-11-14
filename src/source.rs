//! A source file as lines, which is what the padding rules read and their fixes rewrite.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// A line-based change one finding asks for. Lines are 1-based, as spans and editors count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    InsertBlankAfter(usize),
    DeleteLines(Vec<usize>),
    /// Lines `first..=last` become `lines`.
    Replace {
        first: usize,
        last: usize,
        lines: Vec<String>,
    },
}

/// A Rust source file, split into lines.
pub struct Source {
    pub path: PathBuf,
    pub lines: Vec<String>,
    line_break: &'static str,
    trailing_newline: bool,
}

impl Source {
    #[must_use]
    pub fn new(path: PathBuf, text: &str) -> Self {
        let line_break = if text.contains("\r\n") { "\r\n" } else { "\n" };

        Self {
            path,
            lines: text.lines().map(str::to_owned).collect(),
            line_break,
            trailing_newline: text.ends_with('\n'),
        }
    }

    #[must_use]
    pub fn text(&self) -> String {
        let mut text = self.lines.join(self.line_break);

        if self.trailing_newline {
            text.push_str(self.line_break);
        }

        text
    }

    #[must_use]
    pub fn line(&self, number: usize) -> Option<&str> {
        number
            .checked_sub(1)
            .and_then(|index| self.lines.get(index))
            .map(String::as_str)
    }

    #[must_use]
    pub fn is_blank(&self, number: usize) -> bool {
        self.line(number).is_none_or(|text| text.trim().is_empty())
    }

    /// `// bs-allow(<rule>): <reason>` on a finding's line or the line above waives it.
    /// The reason is required: a waiver nobody explained is slop too.
    #[must_use]
    pub fn waives(&self, number: usize, rule: &str) -> bool {
        let marker = format!("bs-allow({rule}):");

        [number.saturating_sub(1), number]
            .into_iter()
            .filter_map(|candidate| self.line(candidate))
            .any(|text| {
                text.split_once(&marker)
                    .is_some_and(|(_, reason)| !reason.trim().is_empty())
            })
    }

    pub fn apply(&mut self, edits: &[Edit]) {
        let mut inserts: BTreeSet<usize> = BTreeSet::new();
        let mut deletes: BTreeSet<usize> = BTreeSet::new();
        let mut replacements: BTreeMap<usize, &[String]> = BTreeMap::new();

        for edit in edits {
            match edit {
                Edit::InsertBlankAfter(number) => {
                    inserts.insert(*number);
                }
                Edit::DeleteLines(numbers) => deletes.extend(numbers.iter().copied()),
                Edit::Replace { first, last, lines } => {
                    deletes.extend(*first..=*last);
                    replacements.insert(*first, lines);
                }
            }
        }

        self.lines = std::mem::take(&mut self.lines)
            .into_iter()
            .zip(1..)
            .flat_map(|(text, number)| {
                let replaced = replacements.get(&number).copied().unwrap_or_default();
                let kept = (!deletes.contains(&number)).then_some(text);
                let blank = inserts.contains(&number).then(String::new);

                replaced.iter().cloned().chain(kept).chain(blank)
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::{Edit, Source};

    fn source(text: &str) -> Source {
        Source::new("sample.rs".into(), text)
    }

    #[test]
    fn a_waiver_needs_a_reason() {
        let text = "// bs-allow(complexity): a parser is one big match\nfn parse() {}\n// bs-allow(complexity):\nfn other() {}\n";
        let sample = source(text);

        assert!(sample.waives(2, "complexity"));
        assert!(!sample.waives(4, "complexity"));
        assert!(!sample.waives(2, "padding"));
    }

    #[test]
    fn edits_insert_and_delete_whole_lines() {
        let mut sample = source("one\ntwo\n\nthree\n");
        sample.apply(&[Edit::InsertBlankAfter(1), Edit::DeleteLines(vec![3])]);

        assert_eq!(sample.text(), "one\n\ntwo\nthree\n");
    }

    #[test]
    fn crlf_files_keep_their_line_breaks() {
        let mut sample = source("one\r\ntwo\r\n");
        sample.apply(&[Edit::InsertBlankAfter(1)]);

        assert_eq!(sample.text(), "one\r\n\r\ntwo\r\n");
    }
}
