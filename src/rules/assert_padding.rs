//! `assert-padding`: in tests, a run of assertions stands apart and has no blank lines inside.
//!
//! The Rust reading of begone-slop's `expect-padding`. The run is the test's verdict, so it
//! reads as one paragraph, fenced off from the setup above it.

use syn::Block;
use syn::visit::{self, Visit};

use super::gap::{forbid_blank, require_blank};
use super::unit::{Kind, Unit};
use super::{FileContext, Finding, Rule};

struct AssertPadding<'run> {
    file: &'run FileContext<'run>,
    findings: Vec<Finding>,
}

impl AssertPadding<'_> {
    fn sequence(&mut self, block: &Block) {
        let units: Vec<Unit> = block.stmts.iter().map(Unit::of_stmt).collect();

        for pair in units.windows(2) {
            let [previous, next] = pair else {
                continue;
            };

            let gap = previous.gap_to(*next);
            let previous_asserts = previous.kind == Kind::Assert;
            let next_asserts = next.kind == Kind::Assert;

            let finding = if previous_asserts && next_asserts {
                forbid_blank(
                    self.file.source,
                    gap,
                    Rule::AssertPadding,
                    "inside a run of assertions",
                )
            } else if previous_asserts || next_asserts {
                require_blank(
                    self.file.source,
                    gap,
                    Rule::AssertPadding,
                    "around a run of assertions",
                )
            } else {
                None
            };

            self.findings.extend(finding);
        }
    }
}

impl<'ast> Visit<'ast> for AssertPadding<'_> {
    fn visit_block(&mut self, block: &'ast Block) {
        if self
            .file
            .is_test(block.brace_token.span.open().start().line)
        {
            self.sequence(block);
        }

        visit::visit_block(self, block);
    }
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut padding = AssertPadding {
        file,
        findings: Vec::new(),
    };

    padding.visit_file(file.syntax);

    padding.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, findings, lines_of};

    #[test]
    fn assertions_are_fenced_off_from_their_setup() {
        let text = "#[test]\nfn adds() {\n    let sum = 1 + 1;\n    assert_eq!(sum, 2);\n}\n";

        assert_eq!(lines_of(Rule::AssertPadding, text), [4]);
    }

    #[test]
    fn a_run_of_assertions_has_no_blank_lines_inside() {
        let text = "#[test]\nfn adds() {\n    let sum = 1 + 1;\n\n    assert_eq!(sum, 2);\n\n    assert!(sum > 0);\n}\n";

        assert_eq!(lines_of(Rule::AssertPadding, text), [7]);
    }

    #[test]
    fn code_outside_tests_is_left_alone() {
        let text = "fn adds() {\n    let sum = 1 + 1;\n    debug_assert_eq!(sum, 2);\n}\n";

        assert_eq!(lines_of(Rule::AssertPadding, text), NONE);
    }

    #[test]
    fn a_tests_directory_counts_as_tests() {
        let text = "fn helper() {\n    let sum = 1 + 1;\n    assert_eq!(sum, 2);\n}\n";
        let found = findings("tests/sample.rs", text);
        let padding = found
            .iter()
            .filter(|finding| finding.rule == Rule::AssertPadding)
            .count();

        assert_eq!(padding, 1);
    }
}
