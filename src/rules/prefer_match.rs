//! `prefer-match`: an `if`/`else if` chain comparing one subject to fixed values is a `match`.
//!
//! begone-slop's `prefer-effect-match`, in Rust's own terms. A chain of three or more
//! `subject == VALUE` tests (or `||` of them) restates a `match`, minus the compiler checking
//! that every case is covered.

use quote::ToTokens;
use syn::visit::{self, Visit};
use syn::{BinOp, Expr, ExprIf};

use super::{Finding, Rule};

const MIN_BRANCHES: usize = 3;

struct PreferMatch {
    findings: Vec<Finding>,
}

impl<'ast> Visit<'ast> for PreferMatch {
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
            self.findings.push(Finding {
                line: branch.if_token.span.start().line,
                rule: Rule::PreferMatch,
                message: format!("this chain compares `{subject}` to fixed values; write it as a `match` so the compiler checks the cases"),
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

/// A literal, or a path such as an enum variant or a constant.
fn is_fixed(expr: &Expr) -> bool {
    matches!(expr, Expr::Lit(_) | Expr::Path(_))
}

pub(super) fn check(file: &syn::File) -> Vec<Finding> {
    let mut prefer_match = PreferMatch {
        findings: Vec::new(),
    };
    prefer_match.visit_file(file);

    prefer_match.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

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
