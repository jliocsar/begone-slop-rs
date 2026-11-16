//! `parenthesize-mixed-operators`: a reader shouldn't need Rust's precedence table.
//!
//! `y & mask ^ bits >> 1 == 0` is `((y & mask) ^ (bits >> 1)) == 0`, but only a reader who
//! knows that `&` binds tighter than `^`, which binds tighter than `==`, can tell. Where a
//! bitwise operator (`&`, `|`, `^`) or a shift (`<<`, `>>`) has another operator under it, or a
//! comparison has a bitwise operator or a shift under it, the inner one goes in parentheses.
//! An `&&` directly under an `||` goes in parentheses too: `a && b || c` is `(a && b) || c`,
//! so write that. Arithmetic under a comparison (`value % 3 != 0`), a comparison under `&&` or
//! `||`, and one operator repeated (`a & b & c`, `a || b || c`) read fine and pass.

use std::mem;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{BinOp, Expr, ExprBinary};

use super::{FileContext, Finding, Rule};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Arithmetic,
    Bitwise,
    Shift,
    Comparison,
    Logical,
}

struct MixedOperators {
    findings: Vec<Finding>,
}

impl<'ast> Visit<'ast> for MixedOperators {
    fn visit_expr_binary(&mut self, expression: &'ast ExprBinary) {
        for operand in [&*expression.left, &*expression.right] {
            if let Expr::Binary(inner) = operand
                && needs_parentheses(&expression.op, &inner.op)
            {
                let line = inner.span().start().line;
                let inner_symbol = symbol(&inner.op);
                let outer_symbol = symbol(&expression.op);

                if self
                    .findings
                    .last()
                    .is_none_or(|last| last.line != line)
                {
                    self.findings.push(Finding {
                        line,
                        rule: Rule::ParenthesizeMixedOperators,
                        message: format!("`{inner_symbol}` sits under `{outer_symbol}` without parentheses; parenthesize it so nobody needs the precedence table"),
                        edit: None,
                    });
                }
            }
        }

        visit::visit_expr_binary(self, expression);
    }
}

fn needs_parentheses(outer: &BinOp, inner: &BinOp) -> bool {
    let repeated = mem::discriminant(outer) == mem::discriminant(inner);

    match (family(outer), family(inner)) {
        (Family::Bitwise | Family::Shift, Family::Arithmetic | Family::Bitwise | Family::Shift) => {
            !repeated
        }
        (Family::Comparison, Family::Bitwise | Family::Shift) => true,
        (Family::Logical, Family::Logical) => {
            matches!((outer, inner), (BinOp::Or(_), BinOp::And(_)))
        }
        _ => false,
    }
}

fn family(operator: &BinOp) -> Family {
    match operator {
        BinOp::Add(_) | BinOp::Sub(_) | BinOp::Mul(_) | BinOp::Div(_) | BinOp::Rem(_) => {
            Family::Arithmetic
        }
        BinOp::BitAnd(_) | BinOp::BitOr(_) | BinOp::BitXor(_) => Family::Bitwise,
        BinOp::Shl(_) | BinOp::Shr(_) => Family::Shift,
        BinOp::Eq(_) | BinOp::Ne(_) | BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_) => {
            Family::Comparison
        }
        _ => Family::Logical,
    }
}

fn symbol(operator: &BinOp) -> &'static str {
    match operator {
        BinOp::Add(_) => "+",
        BinOp::Sub(_) => "-",
        BinOp::Mul(_) => "*",
        BinOp::Div(_) => "/",
        BinOp::Rem(_) => "%",
        BinOp::BitAnd(_) => "&",
        BinOp::BitOr(_) => "|",
        BinOp::BitXor(_) => "^",
        BinOp::Shl(_) => "<<",
        BinOp::Shr(_) => ">>",
        BinOp::Eq(_) => "==",
        BinOp::Ne(_) => "!=",
        BinOp::Lt(_) => "<",
        BinOp::Le(_) => "<=",
        BinOp::Gt(_) => ">",
        BinOp::Ge(_) => ">=",
        BinOp::And(_) => "&&",
        BinOp::Or(_) => "||",
        _ => "an assignment",
    }
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut operators = MixedOperators {
        findings: Vec::new(),
    };

    operators.visit_file(file.syntax);

    operators.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn mixed_bitwise_shift_and_comparison_are_reported() {
        let text = "fn odd(y: i32) -> bool {\n    let a = y & 0b1010 ^ 3;\n    let b = y ^ 0xff >> 1;\n    let c = y * y ^ 0xff;\n    y & 1 == 0\n}\n";

        assert_eq!(
            lines_of(Rule::ParenthesizeMixedOperators, text),
            [2, 3, 4, 5]
        );
    }

    #[test]
    fn parenthesized_repeated_and_arithmetic_comparisons_pass() {
        let text = "fn odd(value: i32) -> bool {\n    let low = (value * value) ^ 0xff;\n    let all = value & MASK & 0xff;\n    let bits = (!0xff_u32 >> 1) as i32;\n\n    (low == bits && value <= 42) || value % 3 != 0\n}\n";

        assert_eq!(lines_of(Rule::ParenthesizeMixedOperators, text), NONE);
    }

    #[test]
    fn and_directly_under_or_is_reported() {
        let text = "fn keep(left: bool, right: bool, other: bool) -> bool {\n    let first = left && right || other;\n    let second = other || left && right;\n    let third = left\n        && right\n        || other;\n\n    first && second && third\n}\n";

        assert_eq!(lines_of(Rule::ParenthesizeMixedOperators, text), [2, 3, 4]);
    }

    #[test]
    fn parenthesized_or_repeated_logic_passes() {
        let text = "fn keep(left: bool, right: bool, other: bool) -> bool {\n    let first = (left && right) || other;\n    let second = left && (right || other);\n    let third = left || right || other;\n\n    first && second && third && left == right\n}\n";

        assert_eq!(lines_of(Rule::ParenthesizeMixedOperators, text), NONE);
    }

    #[test]
    fn one_line_is_reported_once() {
        let text = "fn odd(y: i32) -> bool {\n    y & 0b1010 ^ !0xff >> 1 == 0\n}\n";

        assert_eq!(lines_of(Rule::ParenthesizeMixedOperators, text), [2]);
    }
}
