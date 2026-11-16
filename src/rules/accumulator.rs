//! `no-mutable-accumulator`: a collection or total built by a loop is an iterator chain.
//!
//! `let mut out = Vec::new(); for x in xs { out.push(f(x)) }` is `xs.map(f).collect()`, and a
//! running `+=` is `.sum()` or `.fold()`. The chain says what it builds in one expression;
//! the loop makes the reader replay it. Only the plain shape is reported: an empty start,
//! then straight away a `for` whose body does nothing but grow it (with `if` to filter and
//! `let` to name a step). A loop that also breaks, returns, uses `?` or touches anything
//! else is real control flow and is left alone.

use syn::visit::{self, Visit};
use syn::{BinOp, Block, Expr, ExprForLoop, Local, Pat, Stmt};

use super::{FileContext, Finding, Rule};

const EMPTY_CONSTRUCTORS: [&str; 8] = [
    "BTreeMap", "BTreeSet", "HashMap", "HashSet", "String", "Vec", "VecDeque", "IndexMap",
];

const GROWING_METHODS: [&str; 6] = [
    "extend",
    "insert",
    "push",
    "push_back",
    "push_str",
    "push_front",
];

struct Accumulators {
    findings: Vec<Finding>,
}

impl<'ast> Visit<'ast> for Accumulators {
    fn visit_block(&mut self, block: &'ast Block) {
        for pair in block.stmts.windows(2) {
            if let [Stmt::Local(local), Stmt::Expr(Expr::ForLoop(looping), _)] = pair
                && let Some(name) = empty_accumulator(local)
                && only_grows(&looping.body, &name)
            {
                self.findings.push(Finding {
                    line: local.let_token.span.start().line,
                    rule: Rule::NoMutableAccumulator,
                    message: format!(
                        "`{name}` is built by a loop; build it with an iterator instead (`.map(…).collect()`, `.filter(…)`, `.sum()`, `.fold(…)`)"
                    ),
                    edit: None,
                });
            }
        }

        visit::visit_block(self, block);
    }
}

/// `let mut name = Vec::new()` (or `vec![]`, `String::new()`, `0`, ...): the name.
fn empty_accumulator(local: &Local) -> Option<String> {
    let Pat::Ident(binding) = &local.pat else {
        return None;
    };

    let init = &local.init.as_ref()?.expr;

    let is_empty = match &**init {
        Expr::Call(call) => matches!(&*call.func, Expr::Path(path)
            if path.path.segments.len() >= 2
                && path.path.segments.iter().rev().nth(1).is_some_and(|owner| EMPTY_CONSTRUCTORS.iter().any(|name| owner.ident == name))
                && path.path.segments.last().is_some_and(|method| method.ident == "new" || method.ident == "with_capacity")),
        Expr::Macro(invocation) => {
            invocation.mac.path.is_ident("vec") && invocation.mac.tokens.is_empty()
        }
        Expr::Lit(literal) => {
            matches!(&literal.lit, syn::Lit::Int(number) if number.base10_digits() == "0")
                || matches!(&literal.lit, syn::Lit::Float(number) if number.base10_digits().trim_end_matches('0').trim_end_matches('.') == "0")
        }
        _ => false,
    };

    (binding.mutability.is_some() && is_empty).then(|| binding.ident.to_string())
}

fn only_grows(body: &Block, name: &str) -> bool {
    body.stmts
        .iter()
        .all(|stmt| match stmt {
            Stmt::Local(local) => local
                .init
                .as_ref()
                .is_some_and(|init| init.diverge.is_none() && is_plain(&init.expr)),
            Stmt::Expr(expr, _) => grows(expr, name),
            Stmt::Item(_) | Stmt::Macro(_) => false,
        })
}

fn grows(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::MethodCall(call) => {
            is_named(&call.receiver, name)
                && GROWING_METHODS
                    .iter()
                    .any(|method| call.method == method)
                && call.args.iter().all(is_plain)
        }
        Expr::Binary(binary) => {
            matches!(binary.op, BinOp::AddAssign(_) | BinOp::MulAssign(_))
                && is_named(&binary.left, name)
                && is_plain(&binary.right)
        }
        Expr::If(branch) => {
            is_plain(&branch.cond)
                && only_grows(&branch.then_branch, name)
                && branch
                    .else_branch
                    .as_ref()
                    .is_none_or(|(_, otherwise)| match &**otherwise {
                        Expr::Block(block) => only_grows(&block.block, name),
                        other => grows(other, name),
                    })
        }
        _ => false,
    }
}

fn is_named(expr: &Expr, name: &str) -> bool {
    matches!(expr, Expr::Path(path) if path.path.is_ident(name))
}

/// No `?`, `return`, `break`, `continue` or `await` anywhere inside.
fn is_plain(expr: &Expr) -> bool {
    struct Jumps(bool);

    impl<'ast> Visit<'ast> for Jumps {
        fn visit_expr(&mut self, expr: &'ast Expr) {
            if matches!(
                expr,
                Expr::Try(_)
                    | Expr::Return(_)
                    | Expr::Break(_)
                    | Expr::Continue(_)
                    | Expr::Await(_)
            ) {
                self.0 = true;
            }

            visit::visit_expr(self, expr);
        }

        fn visit_expr_for_loop(&mut self, _nested: &'ast ExprForLoop) {}
    }

    let mut jumps = Jumps(false);

    jumps.visit_expr(expr);

    !jumps.0
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut accumulators = Accumulators {
        findings: Vec::new(),
    };

    accumulators.visit_file(file.syntax);

    accumulators.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn a_vec_built_by_pushing_is_reported() {
        let text = "fn names(users: &[User]) -> Vec<String> {\n    let mut names = Vec::new();\n    for user in users {\n        names.push(user.name.clone());\n    }\n\n    names\n}\n";

        assert_eq!(lines_of(Rule::NoMutableAccumulator, text), [2]);
    }

    #[test]
    fn a_filtered_sum_is_reported() {
        let text = "fn total(items: &[Item]) -> u32 {\n    let mut total = 0;\n    for item in items {\n        let price = item.price;\n        if item.active {\n            total += price;\n        }\n    }\n\n    total\n}\n";

        assert_eq!(lines_of(Rule::NoMutableAccumulator, text), [2]);
    }

    #[test]
    fn a_loop_with_real_control_flow_is_left_alone() {
        let text = "fn load(paths: &[Path]) -> Result<Vec<String>, Error> {\n    let mut texts = Vec::new();\n    for path in paths {\n        texts.push(read(path)?);\n    }\n\n    Ok(texts)\n}\n";

        assert_eq!(lines_of(Rule::NoMutableAccumulator, text), NONE);
    }

    #[test]
    fn a_loop_that_does_more_is_left_alone() {
        let text = "fn run(items: &[Item]) {\n    let mut seen = Vec::new();\n    for item in items {\n        seen.push(item.id);\n        log(item);\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoMutableAccumulator, text), NONE);
    }
}
