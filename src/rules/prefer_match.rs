//! `prefer-match`: an `if`/`else if` chain comparing one subject to fixed values is a `match`.
//!
//! begone-slop's `prefer-effect-match`, in Rust's own terms. A chain of three or more
//! `subject == VALUE` tests (or `||` of them) restates a `match`, minus the compiler checking
//! that every case is covered. When the values are strings, a `match` on them is what
//! `no-stringly-match` rejects, so outside a parse (`FromStr`, `TryFrom`, `parse*`, `from_*`) the
//! advice is the same as that rule's: parse into an enum.

use quote::ToTokens;
use syn::visit::{self, Visit};
use syn::{BinOp, Expr, ExprIf, ImplItemFn, ItemFn, ItemImpl, Lit};

use super::{Finding, Rule};

const MIN_BRANCHES: usize = 3;
const PARSE_PREFIXES: [&str; 2] = ["from_", "parse"];
const PARSE_TRAITS: [&str; 2] = ["FromStr", "TryFrom"];

/// What a chain compares its subject to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Values {
    Fixed,
    Strings,
}

struct PreferMatch {
    findings: Vec<Finding>,
    parse_depth: usize,
}

impl PreferMatch {
    fn message(&self, subject: &str, values: Values) -> String {
        if values == Values::Strings && self.parse_depth == 0 {
            format!(
                "this chain compares `{subject}` to strings; parse it into an enum once, at the boundary, and match on that"
            )
        } else {
            format!(
                "this chain compares `{subject}` to fixed values; write it as a `match` so the compiler checks the cases"
            )
        }
    }
}

impl<'ast> Visit<'ast> for PreferMatch {
    fn visit_impl_item_fn(&mut self, method: &'ast ImplItemFn) {
        let parses = is_parse_name(&method.sig.ident.to_string());
        self.parse_depth += usize::from(parses);
        visit::visit_impl_item_fn(self, method);
        self.parse_depth -= usize::from(parses);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let parses = is_parse_name(&function.sig.ident.to_string());
        self.parse_depth += usize::from(parses);
        visit::visit_item_fn(self, function);
        self.parse_depth -= usize::from(parses);
    }

    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        let parses = implementation
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .is_some_and(|segment| PARSE_TRAITS.iter().any(|name| segment.ident == name));
        self.parse_depth += usize::from(parses);
        visit::visit_item_impl(self, implementation);
        self.parse_depth -= usize::from(parses);
    }

    fn visit_expr_if(&mut self, branch: &'ast ExprIf) {
        let subjects: Option<Vec<String>> = chain(branch)
            .iter()
            .map(|link| subject_of(&link.cond))
            .collect();
        let one_subject = subjects.as_ref().is_some_and(|subjects| {
            subjects.len() >= MIN_BRANCHES
                && subjects.windows(2).all(|pair| pair.first() == pair.get(1))
        });

        if one_subject
            && let Some(subject) = subjects.as_ref().and_then(|subjects| subjects.first())
        {
            let strings = chain(branch)
                .iter()
                .any(|link| compares_strings(&link.cond));

            let values = if strings {
                Values::Strings
            } else {
                Values::Fixed
            };

            self.findings.push(Finding {
                line: branch.if_token.span.start().line,
                rule: Rule::PreferMatch,
                message: self.message(subject, values),
                edit: None,
            });
        }

        // The rest of the chain is visited through its links, not as chains of their own.
        for link in chain(branch) {
            self.visit_expr(&link.cond);
            self.visit_block(&link.then_branch);
        }

        if let Some(last) = chain(branch).last()
            && let Some((_, otherwise)) = &last.else_branch
        {
            visit::visit_expr(self, otherwise);
        }
    }
}

/// `if a {} else if b {} else if c {}` as `[a, b, c]`.
fn chain(first: &ExprIf) -> Vec<&ExprIf> {
    std::iter::successors(Some(first), |link| {
        match link.else_branch.as_ref().map(|(_, otherwise)| &**otherwise) {
            Some(Expr::If(next)) => Some(next),
            _ => None,
        }
    })
    .collect()
}

/// `subject == VALUE` or `subject == A || subject == B`, as the subject's text.
fn subject_of(condition: &Expr) -> Option<String> {
    let Expr::Binary(binary) = condition else {
        return None;
    };

    match binary.op {
        BinOp::Eq(_) if is_fixed(&binary.right) => Some(binary.left.to_token_stream().to_string()),
        BinOp::Or(_) => {
            let left = subject_of(&binary.left)?;

            (subject_of(&binary.right)? == left).then_some(left)
        }
        _ => None,
    }
}

/// Whether any value the condition compares against is a string literal.
fn compares_strings(condition: &Expr) -> bool {
    match condition {
        Expr::Binary(binary) => match binary.op {
            BinOp::Eq(_) => {
                matches!(&*binary.right, Expr::Lit(literal) if matches!(literal.lit, Lit::Str(_)))
            }
            BinOp::Or(_) => compares_strings(&binary.left) || compares_strings(&binary.right),
            _ => false,
        },
        _ => false,
    }
}

fn is_parse_name(name: &str) -> bool {
    PARSE_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// A literal, or a path such as an enum variant or a constant.
fn is_fixed(expr: &Expr) -> bool {
    matches!(expr, Expr::Lit(_) | Expr::Path(_))
}

pub(super) fn check(file: &syn::File) -> Vec<Finding> {
    let mut prefer_match = PreferMatch {
        findings: Vec::new(),
        parse_depth: 0,
    };
    prefer_match.visit_file(file);

    prefer_match.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, findings, lines_of};

    fn messages(text: &str) -> Vec<String> {
        findings("src/sample.rs", text)
            .into_iter()
            .filter(|finding| finding.rule == Rule::PreferMatch)
            .map(|finding| finding.message)
            .collect()
    }

    #[test]
    fn a_string_chain_is_told_to_parse_into_an_enum() {
        let chain = "    if name == \"fast\" {\n        3\n    } else if name == \"slow\" {\n        1\n    } else if name == \"off\" {\n        0\n    } else {\n        2\n    }\n";
        let text = format!(
            "fn speed(name: &str) -> u8 {{\n{chain}}}\n\nfn parse_speed(name: &str) -> u8 {{\n{chain}}}\n"
        );

        assert_eq!(
            messages(&text),
            [
                "this chain compares `name` to strings; parse it into an enum once, at the boundary, and match on that",
                "this chain compares `name` to fixed values; write it as a `match` so the compiler checks the cases",
            ]
        );
    }

    #[test]
    fn a_chain_of_equality_tests_is_a_match() {
        let text = "fn speed(mode: Mode) -> u8 {\n    if mode == Mode::Fast {\n        3\n    } else if mode == Mode::Slow || mode == Mode::Idle {\n        1\n    } else if mode == Mode::Off {\n        0\n    } else {\n        2\n    }\n}\n";

        assert_eq!(lines_of(Rule::PreferMatch, text), [2]);
    }

    #[test]
    fn two_branches_stay_an_if() {
        let text = "fn speed(mode: Mode) -> u8 {\n    if mode == Mode::Fast {\n        3\n    } else if mode == Mode::Slow {\n        1\n    } else {\n        2\n    }\n}\n";

        assert_eq!(lines_of(Rule::PreferMatch, text), NONE);
    }

    #[test]
    fn different_subjects_stay_an_if() {
        let text = "fn pick(left: u8, right: u8) -> u8 {\n    if left == 1 {\n        1\n    } else if right == 2 {\n        2\n    } else if left == 3 {\n        3\n    } else {\n        0\n    }\n}\n";

        assert_eq!(lines_of(Rule::PreferMatch, text), NONE);
    }
}
