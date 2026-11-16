//! `padding`: blank lines between statements and items, by what they are.
//!
//! The Rust reading of begone-slop's `padding-line-between-statements` preset. Blocks, items,
//! `let`s and returns stand apart; imports end with a blank line; one-line constants of the same
//! visibility form a group with none inside it. rustfmt keeps blank lines it finds, so the
//! fixes here and rustfmt never undo each other.

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::visit::{self, Visit};
use syn::{Block, File, ItemImpl, ItemMacro, ItemMod, ItemTrait, Macro};

use super::gap::{forbid_blank, require_blank};
use super::unit::{Kind, Unit};
use super::{FileContext, Finding, Rule};
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
        self.sequence(Unit::of_block(block).into_iter());
        visit::visit_block(self, block);
    }

    fn visit_file(&mut self, file: &'ast File) {
        self.sequence(file.items.iter().map(Unit::of_item));
        visit::visit_file(self, file);
    }

    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        self.sequence(
            implementation
                .items
                .iter()
                .map(Unit::of_impl_item),
        );
        visit::visit_item_impl(self, implementation);
    }

    /// A `macro_rules!` body is a template, not code: its `$` fragments only become code
    /// where the macro is used.
    fn visit_item_macro(&mut self, definition: &'ast ItemMacro) {
        if definition.ident.is_none() {
            visit::visit_item_macro(self, definition);
        }
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if let Some((_, items)) = &module.content {
            self.sequence(items.iter().map(Unit::of_item));
        }

        visit::visit_item_mod(self, module);
    }

    fn visit_item_trait(&mut self, definition: &'ast ItemTrait) {
        self.sequence(
            definition
                .items
                .iter()
                .map(Unit::of_trait_item),
        );
        visit::visit_item_trait(self, definition);
    }

    /// `syn` leaves a macro's arguments as tokens, so the code in `tokio::select!` arms and
    /// in closures passed to `assert!` would go unchecked. Every braced group in them that
    /// parses as a block is one.
    fn visit_macro(&mut self, invocation: &'ast Macro) {
        self.blocks_in(invocation.tokens.clone());
    }
}

impl Padding<'_> {
    fn blocks_in(&mut self, tokens: TokenStream) {
        for tree in tokens {
            let TokenTree::Group(group) = tree else {
                continue;
            };

            let parsed = (group.delimiter() == Delimiter::Brace)
                .then(|| syn::parse2::<Block>(TokenTree::Group(group.clone()).into()).ok())
                .flatten();

            match parsed {
                Some(block) => self.visit_block(&block),
                None => self.blocks_in(group.stream()),
            }
        }
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
        (_, Kind::Tail) => Blank::Always("before the block's value"),
        (Kind::BlockLike | Kind::ItemBlock, _) => Blank::Always("after a block"),
        (_, Kind::BlockLike | Kind::ItemBlock) => Blank::Always("before a block"),
        // A run of assertions and what sits around it is `assert-padding`'s call.
        (Kind::Assert, _) | (_, Kind::Assert) | (Kind::Let, Kind::Let) => Blank::Any,
        (Kind::Let, _) => Blank::Always("after the `let`s"),
        (_, Kind::Let) => Blank::Always("before a `let`"),
        _ => Blank::Any,
    }
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut padding = Padding {
        source: file.source,
        findings: Vec::new(),
    };

    padding.visit_file(file.syntax);

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
    fn lets_sit_together_and_stand_apart_from_the_rest() {
        let text = "fn run() {\n    let left = 1;\n    let right = 2;\n    add(left, right);\n    let done = true;\n}\n";

        assert_eq!(lines_of(Rule::Padding, text), [4, 5]);
    }

    #[test]
    fn a_blocks_value_stands_apart_like_a_return() {
        let text = "fn run() -> u8 {\n    prepare();\n    total()\n}\n";

        assert_eq!(lines_of(Rule::Padding, text), [3]);
    }

    #[test]
    fn a_block_with_a_call_hung_off_it_is_still_a_block() {
        let text = "fn run(mode: u8) {\n    prepare();\n    match mode {\n        _ => app(),\n    }\n    .run();\n}\n";

        assert_eq!(lines_of(Rule::Padding, text), [3]);
    }

    #[test]
    fn blocks_inside_macro_arguments_are_checked() {
        let text = "fn run() {\n    select! {\n        value = next() => {\n            let ready = value;\n            go(ready);\n        }\n    }\n}\n";

        assert_eq!(lines_of(Rule::Padding, text), [5]);
    }

    #[test]
    fn a_blank_line_below_the_comment_does_not_count() {
        let text = "fn run() {\n    prepare();\n    // Then the rest.\n\n    let done = true;\n}\n";

        assert_eq!(lines_of(Rule::Padding, text), [5]);
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
