//! `complexity`: cyclomatic complexity per function, capped at 15 as in the TypeScript preset.
//!
//! Counts one path plus one per decision: `if`, each `match` arm past the first and each guard,
//! loops, `&&`, `||` and `let ... else`. `?` isn't counted: it's how Rust spells "and then", and
//! counting it would punish the code that handles every error. Closures count toward the
//! function they sit in; nested `fn` items are measured on their own.

use syn::visit::{self, Visit};
use syn::{
    BinOp, Block, Expr, ExprBinary, ExprForLoop, ExprIf, ExprLoop, ExprMatch, ExprWhile, File,
    Ident, ImplItemFn, Item, ItemFn, Local, TraitItemFn,
};

use super::{Finding, Rule};

const MAX_COMPLEXITY: usize = 15;

struct Functions {
    findings: Vec<Finding>,
}

struct Branches {
    count: usize,
}

impl Functions {
    fn measure(&mut self, name: &Ident, body: &Block) {
        let mut branches = Branches { count: 1 };
        branches.visit_block(body);

        if branches.count > MAX_COMPLEXITY {
            self.findings.push(Finding {
                line: name.span().start().line,
                rule: Rule::Complexity,
                message: format!(
                    "`{name}` has a cyclomatic complexity of {} (max {MAX_COMPLEXITY}); move some decisions into functions that name them",
                    branches.count
                ),
                edit: None,
            });
        }
    }
}

impl<'ast> Visit<'ast> for Functions {
    fn visit_impl_item_fn(&mut self, method: &'ast ImplItemFn) {
        self.measure(&method.sig.ident, &method.block);
        visit::visit_impl_item_fn(self, method);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.measure(&function.sig.ident, &function.block);
        visit::visit_item_fn(self, function);
    }

    fn visit_trait_item_fn(&mut self, method: &'ast TraitItemFn) {
        if let Some(body) = &method.default {
            self.measure(&method.sig.ident, body);
        }

        visit::visit_trait_item_fn(self, method);
    }
}

impl<'ast> Visit<'ast> for Branches {
    fn visit_expr_binary(&mut self, binary: &'ast ExprBinary) {
        self.count += usize::from(matches!(binary.op, BinOp::And(_) | BinOp::Or(_)));
        visit::visit_expr_binary(self, binary);
    }

    fn visit_expr_for_loop(&mut self, looping: &'ast ExprForLoop) {
        self.count += 1;
        visit::visit_expr_for_loop(self, looping);
    }

    fn visit_expr_if(&mut self, branch: &'ast ExprIf) {
        self.count += 1;
        visit::visit_expr_if(self, branch);
    }

    fn visit_expr_loop(&mut self, looping: &'ast ExprLoop) {
        self.count += 1;
        visit::visit_expr_loop(self, looping);
    }

    fn visit_expr_match(&mut self, matching: &'ast ExprMatch) {
        let guards = matching
            .arms
            .iter()
            .filter(|arm| arm.guard.is_some())
            .count();
        let lookup = guards == 0 && matching.arms.iter().all(|arm| is_value(&arm.body));

        if !lookup {
            self.count += matching.arms.len().saturating_sub(1) + guards;
        }

        visit::visit_expr_match(self, matching);
    }

    fn visit_expr_while(&mut self, looping: &'ast ExprWhile) {
        self.count += 1;
        visit::visit_expr_while(self, looping);
    }

    fn visit_item(&mut self, _nested: &'ast Item) {}

    fn visit_local(&mut self, local: &'ast Local) {
        let diverges = local
            .init
            .as_ref()
            .is_some_and(|init| init.diverge.is_some());
        self.count += usize::from(diverges);
        visit::visit_local(self, local);
    }
}

/// A literal, a path, or a constructor call on values: an arm that looks something up.
fn is_value(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(_) | Expr::Path(_) => true,
        Expr::Reference(reference) => is_value(&reference.expr),
        Expr::Call(call) => matches!(&*call.func, Expr::Path(_)) && call.args.iter().all(is_value),
        _ => false,
    }
}

pub(super) fn check(file: &File) -> Vec<Finding> {
    let mut functions = Functions {
        findings: Vec::new(),
    };
    functions.visit_file(file);

    functions.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    fn function_with_ifs(count: usize) -> String {
        let body = "    if ready() { go(); }\n".repeat(count);

        format!("fn run() {{\n{body}}}\n")
    }

    #[test]
    fn fifteen_paths_pass() {
        assert_eq!(lines_of(Rule::Complexity, &function_with_ifs(14)), NONE);
    }

    #[test]
    fn the_sixteenth_path_is_reported_on_the_name() {
        assert_eq!(lines_of(Rule::Complexity, &function_with_ifs(15)), [1]);
    }

    #[test]
    fn match_arms_past_the_first_count() {
        let arms = (0..16)
            .map(|number| format!("        {number} => go(value).await,\n"))
            .collect::<Vec<_>>()
            .concat();
        let text = format!(
            "fn run(value: u8) {{\n    match value {{\n{arms}        _ => stop(),\n    }}\n}}\n"
        );

        assert_eq!(lines_of(Rule::Complexity, &text), [1]);
    }

    #[test]
    fn a_lookup_match_counts_once() {
        let arms = (0..30)
            .map(|number| format!("        {number} => \"name\",\n"))
            .collect::<Vec<_>>()
            .concat();
        let text = format!(
            "fn name(value: u8) -> &'static str {{\n    match value {{\n{arms}        _ => \"other\",\n    }}\n}}\n"
        );

        assert_eq!(lines_of(Rule::Complexity, &text), NONE);
    }

    #[test]
    fn a_waiver_with_a_reason_silences_it() {
        let text = format!(
            "// bs-allow(complexity): one arm per opcode reads best\n{}",
            function_with_ifs(15)
        );

        assert_eq!(lines_of(Rule::Complexity, &text), NONE);
    }
}
