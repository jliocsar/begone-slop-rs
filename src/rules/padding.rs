//! `padding`: blank lines between statements and items, by what they are.
//!
//! The Rust reading of begone-slop's `padding-line-between-statements` preset. Blocks, items
//! and returns stand apart; imports end with a blank line; one-line constants of the same
//! visibility form a group with none inside it. rustfmt keeps blank lines it finds, so the
//! fixes here and rustfmt never undo each other.

use syn::visit::{self, Visit};
use syn::{Block, File, ItemImpl, ItemMod, ItemTrait};

use super::gap::{forbid_blank, require_blank};
use super::unit::{Kind, Unit};
use super::{Finding, Rule};
use crate::source::Source;

enum Blank {
    Always(&'static str),
    Never(&'static str),
    Any,
}

struct Padding<'source> {
    source: &'source Source,
    findings: Vec<Finding>,
}

impl Padding<'_> {
    fn sequence(&mut self, units: impl Iterator<Item = Unit>) {
        let units: Vec<Unit> = units.collect();

        for pair in units.windows(2) {
            let [previous, next] = pair else {
                continue;
            };

            let gap = previous.gap_to(*next);

            let finding = match expected(previous.kind, next.kind) {
                Blank::Always(reason) => require_blank(self.source, gap, Rule::Padding, reason),
                Blank::Never(reason) => forbid_blank(self.source, gap, Rule::Padding, reason),
                Blank::Any => None,
            };

            self.findings.extend(finding);
        }
    }
}

impl<'ast> Visit<'ast> for Padding<'_> {
    fn visit_block(&mut self, block: &'ast Block) {
        self.sequence(block.stmts.iter().map(Unit::of_stmt));
        visit::visit_block(self, block);
    }

    fn visit_file(&mut self, file: &'ast File) {
        self.sequence(file.items.iter().map(Unit::of_item));
        visit::visit_file(self, file);
    }

    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        self.sequence(implementation.items.iter().map(Unit::of_impl_item));
        visit::visit_item_impl(self, implementation);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if let Some((_, items)) = &module.content {
            self.sequence(items.iter().map(Unit::of_item));
        }

        visit::visit_item_mod(self, module);
    }

    fn visit_item_trait(&mut self, definition: &'ast ItemTrait) {
        self.sequence(definition.items.iter().map(Unit::of_trait_item));
        visit::visit_item_trait(self, definition);
    }
}

/// The first arm that matches wins, so the narrow cases sit above the broad ones.
fn expected(previous: Kind, next: Kind) -> Blank {
    match (previous, next) {
        (Kind::Const { public: left }, Kind::Const { public: right }) if left == right => {
            Blank::Never("inside a group of constants")
        }
        (Kind::Const { .. }, _) | (_, Kind::Const { .. }) => {
            Blank::Always("around a group of constants")
        }
        (Kind::Use, Kind::Use) => Blank::Any,
        (Kind::Use, _) => Blank::Always("after the imports"),
        (_, Kind::Return) => Blank::Always("before a return"),
        (Kind::BlockLike | Kind::ItemBlock, _) => Blank::Always("after a block"),
        (_, Kind::BlockLike | Kind::ItemBlock) => Blank::Always("before a block"),
        _ => Blank::Any,
    }
}

pub(super) fn check(source: &Source, file: &File) -> Vec<Finding> {
    let mut padding = Padding {
        source,
        findings: Vec::new(),
    };
    padding.visit_file(file);

    padding.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, fixed, lines_of};

    #[test]
    fn a_return_after_a_statement_needs_a_blank_line() {
        let text = "fn total() -> u8 {\n    let sum = 1;\n    return sum;\n}\n";

        assert_eq!(lines_of(Rule::Padding, text), [3]);
    }

    #[test]
    fn a_block_stands_apart_from_its_neighbours() {
        let text = "fn run() {\n    let ready = true;\n    if ready {\n        go();\n    }\n    stop();\n}\n";

        assert_eq!(lines_of(Rule::Padding, text), [3, 6]);
    }

    #[test]
    fn one_line_lets_may_sit_together() {
        let text = "fn run() {\n    let left = 1;\n    let right = 2;\n    add(left, right);\n}\n";

        assert_eq!(lines_of(Rule::Padding, text), NONE);
    }

    #[test]
    fn a_multi_line_match_binding_counts_as_a_block() {
        let text = "fn run(flag: bool) {\n    let word = match flag {\n        true => 1,\n        false => 2,\n    };\n    say(word);\n}\n";

        assert_eq!(lines_of(Rule::Padding, text), [6]);
    }

    #[test]
    fn imports_end_with_a_blank_line() {
        let text = "use std::fs;\nuse std::io;\nconst LIMIT: u8 = 1;\n";

        assert_eq!(lines_of(Rule::Padding, text), [3]);
    }

    #[test]
    fn constants_of_one_visibility_group_without_blank_lines() {
        let text = "const LEFT: u8 = 1;\n\nconst RIGHT: u8 = 2;\npub const SHOWN: u8 = 3;\n";

        assert_eq!(lines_of(Rule::Padding, text), [3, 4]);
    }

    #[test]
    fn a_comment_between_constants_keeps_its_blank_line() {
        let text = "const LEFT: u8 = 1;\n\n// The right one.\nconst RIGHT: u8 = 2;\n";

        assert_eq!(lines_of(Rule::Padding, text), NONE);
    }

    #[test]
    fn functions_need_blank_lines_between_them() {
        let text = "fn one() {}\nfn two() {}\n";

        assert_eq!(fixed(text), "fn one() {}\n\nfn two() {}\n");
    }

    #[test]
    fn the_blank_line_goes_above_a_leading_comment() {
        let text = "fn run() {\n    let sum = 1;\n    // Done.\n    return sum;\n}\n";

        assert_eq!(
            fixed(text),
            "fn run() {\n    let sum = 1;\n\n    // Done.\n    return sum;\n}\n"
        );
    }

    #[test]
    fn a_doc_comment_belongs_to_its_item() {
        let text = "fn one() {}\n\n/// Two.\nfn two() {}\n";

        assert_eq!(lines_of(Rule::Padding, text), NONE);
    }
}
