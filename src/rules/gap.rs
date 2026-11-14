//! The lines between two neighbouring statements or items, and the two verdicts on them.

use super::{Finding, Rule};
use crate::source::{Edit, Source};

/// The lines strictly between the end of one unit and the start of the next.
#[derive(Clone, Copy, Debug)]
pub struct Gap {
    pub after: usize,
    pub before: usize,
}

impl Gap {
    fn lines(self) -> impl Iterator<Item = usize> {
        self.after + 1..self.before
    }

    fn has_blank(self, source: &Source) -> bool {
        self.lines().any(|number| source.is_blank(number))
    }

    /// A comment in the gap introduces what follows it, so a blank line around it is the
    /// author's call.
    fn has_comment(self, source: &Source) -> bool {
        self.lines().any(|number| !source.is_blank(number))
    }
}

pub fn require_blank(source: &Source, gap: Gap, rule: Rule, reason: &str) -> Option<Finding> {
    if gap.after >= gap.before || gap.has_blank(source) {
        return None;
    }

    Some(Finding {
        line: gap.before,
        rule,
        message: format!("add a blank line {reason}"),
        edit: Some(Edit::InsertBlankAfter(gap.after)),
    })
}

pub fn forbid_blank(source: &Source, gap: Gap, rule: Rule, reason: &str) -> Option<Finding> {
    if gap.has_comment(source) {
        return None;
    }

    let blanks: Vec<usize> = gap.lines().collect();

    if blanks.is_empty() {
        return None;
    }

    Some(Finding {
        line: gap.before,
        rule,
        message: format!("remove the blank line {reason}"),
        edit: Some(Edit::DeleteLines(blanks)),
    })
}
