//! `no-silent-err-arm`, `no-filter-map-ok` and `no-discard-assign`: an error is handled or
//! passed on, never dropped where nobody sees it.
//!
//! `Err(_) => {}` (or `continue`, `None`, `()`, `Default::default()`), an `if let Err(_)` with an
//! empty body, `.filter_map(Result::ok)` and `.flatten()` over a directory listing all make a
//! failure vanish. `_ = fallible();` does too: clippy's `let_underscore_must_use` sees
//! `let _ =`, and rustc's `unused_must_use` sees neither. `let _ = name;` only silences an
//! unused binding. Test code is left alone: a test fails loudly on its own.

use syn::visit::{self, Visit};
use syn::{Arm, Block, Expr, ExprAssign, ExprIf, ExprMethodCall, Local, Pat, Stmt};

use super::test_scope::TestLines;
use super::{FileContext, Finding, Rule};

/// Calls whose result is a `Result` per item, so `.flatten()` on them drops each error.
const FALLIBLE_ITEMS: [&str; 2] = ["read_dir", "lines"];

struct SilentErrors<'run> {
    tests: &'run TestLines,
    findings: Vec<Finding>,
}

impl SilentErrors<'_> {
    fn push(&mut self, line: usize, rule: Rule, message: &str) {
        if self.tests.contains(line) {
            return;
        }

        self.findings.push(Finding {
            line,
            rule,
            message: message.to_owned(),
            edit: None,
        });
    }

    fn arm(&mut self, arm: &Arm) {
        if ignores_error(&arm.pat) && arm.guard.is_none() && is_silent(&arm.body) {
            self.push(
                line_of_pattern(&arm.pat),
                Rule::NoSilentErrorArm,
                "this arm drops the error without a word; propagate it with `?`, or log it and say why going on is right",
            );
        }
    }

    fn if_let(&mut self, branch: &ExprIf) {
        let Expr::Let(binding) = &*branch.cond else {
            return;
        };

        if ignores_error(&binding.pat)
            && branch.else_branch.is_none()
            && is_silent_block(&branch.then_branch)
        {
            self.push(
                branch.if_token.span.start().line,
                Rule::NoSilentErrorArm,
                "this `if let Err(_)` drops the error without a word; propagate it with `?`, or log it and say why going on is right",
            );
        }
    }

    fn method(&mut self, call: &ExprMethodCall) {
        let method = call.method.to_string();
        let line = call.method.span().start().line;
        let keeps_ok = matches!(method.as_str(), "filter_map" | "flat_map")
            && call.args.len() == 1
            && call
                .args
                .first()
                .is_some_and(only_keeps_ok);
        let flattens_results =
            method == "flatten" && call.args.is_empty() && yields_results(&call.receiver);

        if keeps_ok || flattens_results {
            self.push(
                line,
                Rule::NoFilterMapOk,
                "every failed item vanishes here; collect into `Result<Vec<_>, _>` and use `?`, or `partition` and report the errors",
            );
        }
    }
}

impl<'ast> Visit<'ast> for SilentErrors<'_> {
    fn visit_arm(&mut self, arm: &'ast Arm) {
        self.arm(arm);
        visit::visit_arm(self, arm);
    }

    fn visit_expr_assign(&mut self, assign: &'ast ExprAssign) {
        if matches!(&*assign.left, Expr::Infer(_)) {
            self.push(
                assign.eq_token.span.start().line,
                Rule::NoDiscardAssign,
                "`_ = ...` throws the result away where no lint looks; handle it, propagate it with `?`, or say why it can't fail",
            );
        }

        visit::visit_expr_assign(self, assign);
    }

    fn visit_expr_if(&mut self, branch: &'ast ExprIf) {
        self.if_let(branch);
        visit::visit_expr_if(self, branch);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        self.method(call);
        visit::visit_expr_method_call(self, call);
    }

    fn visit_local(&mut self, local: &'ast Local) {
        let silences = matches!(local.pat, Pat::Wild(_))
            && local.init.as_ref().is_some_and(|init| {
                init.diverge.is_none()
                    && matches!(&*init.expr, Expr::Path(path) if path.path.get_ident().is_some())
            });

        if silences {
            self.push(
                local.let_token.span.start().line,
                Rule::NoDiscardAssign,
                "`let _ = name;` only hides an unused binding; use it, or remove what produces it",
            );
        }

        visit::visit_local(self, local);
    }
}

/// `Err(_)`, `Err(_error)` or `Err(..)`: an error pattern that binds nothing it uses.
fn ignores_error(pattern: &Pat) -> bool {
    let Pat::TupleStruct(tuple) = pattern else {
        return false;
    };

    let is_error_pattern = tuple
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Err");

    let ignored = match tuple.elems.first() {
        Some(Pat::Wild(_) | Pat::Rest(_)) => true,
        Some(Pat::Ident(binding)) => binding
            .ident
            .to_string()
            .starts_with('_'),
        _ => false,
    };

    is_error_pattern && tuple.elems.len() == 1 && ignored
}

fn line_of_pattern(pattern: &Pat) -> usize {
    match pattern {
        Pat::TupleStruct(tuple) => tuple
            .path
            .segments
            .first()
            .map_or(0, |segment| segment.ident.span().start().line),
        _ => 0,
    }
}

/// `{}`, `()`, `continue`, `break`, `None`, `return`, `return Ok(())`, `return None`,
/// `Default::default()`: a body that carries on as if nothing failed.
fn is_silent(body: &Expr) -> bool {
    match body {
        Expr::Block(block) => is_silent_block(&block.block),
        Expr::Tuple(tuple) => tuple.elems.is_empty(),
        Expr::Continue(_) => true,
        Expr::Break(stop) => stop.expr.is_none(),
        Expr::Return(stop) => stop
            .expr
            .as_deref()
            .is_none_or(is_empty_value),
        Expr::Path(path) => path.path.is_ident("None"),
        Expr::Call(call) => {
            call.args.is_empty()
                && matches!(&*call.func, Expr::Path(path)
                    if path.path.segments.last().is_some_and(|segment| segment.ident == "default"))
        }
        _ => false,
    }
}

/// `Ok(())` or `None`.
fn is_empty_value(value: &Expr) -> bool {
    match value {
        Expr::Path(path) => path.path.is_ident("None"),
        Expr::Call(call) => {
            matches!(&*call.func, Expr::Path(path) if path.path.is_ident("Ok"))
                && call.args.len() == 1
                && call.args.first().is_some_and(
                    |argument| matches!(argument, Expr::Tuple(unit) if unit.elems.is_empty()),
                )
        }
        _ => false,
    }
}

fn is_silent_block(block: &Block) -> bool {
    match block.stmts.as_slice() {
        [] => true,
        [Stmt::Expr(expr, _)] => is_silent(expr),
        _ => false,
    }
}

/// `Result::ok`, or a closure whose body ends in `.ok()`.
fn only_keeps_ok(argument: &Expr) -> bool {
    match argument {
        Expr::Path(path) => {
            let names: Vec<String> = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();

            names.ends_with(&["Result".to_owned(), "ok".to_owned()])
        }
        Expr::Closure(closure) => ends_in_ok(&closure.body),
        _ => false,
    }
}

fn ends_in_ok(body: &Expr) -> bool {
    match body {
        Expr::MethodCall(call) => call.method == "ok" && call.args.is_empty(),
        Expr::Block(block) => {
            matches!(block.block.stmts.last(), Some(Stmt::Expr(last, None)) if ends_in_ok(last))
        }
        _ => false,
    }
}

/// A receiver whose items are `Result`s: `read_dir(..)`, `read_dir(..)?`, `reader.lines()`.
fn yields_results(receiver: &Expr) -> bool {
    match receiver {
        Expr::Try(inner) => yields_results(&inner.expr),
        Expr::Call(call) => matches!(&*call.func, Expr::Path(path)
            if path.path.segments.last().is_some_and(|segment| FALLIBLE_ITEMS.iter().any(|name| segment.ident == name))),
        Expr::MethodCall(call) => FALLIBLE_ITEMS
            .iter()
            .any(|name| call.method == name),
        _ => false,
    }
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let syntax = file.syntax;
    let mut silent = SilentErrors {
        tests: &file.tests,
        findings: Vec::new(),
    };

    silent.visit_file(syntax);

    silent.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_in, lines_of};

    #[test]
    fn silent_error_arms_are_reported() {
        let text = "fn load(rows: Rows) -> Option<u8> {\n    for row in rows {\n        match parse(row) {\n            Ok(value) => keep(value),\n            Err(_) => continue,\n        }\n        match parse(row) {\n            Ok(value) => keep(value),\n            Err(_error) => {}\n        }\n        let fallback = match parse(row) {\n            Ok(value) => value,\n            Err(..) => Default::default(),\n        };\n        if let Err(_) = save(row) {}\n    }\n\n    match parse(LAST) {\n        Ok(value) => Some(value),\n        Err(_) => None,\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoSilentErrorArm, text), [5, 9, 13, 15, 20]);
    }

    #[test]
    fn handled_errors_pass() {
        let text = "fn load(row: Row) -> Result<u8, Error> {\n    match parse(row) {\n        Ok(value) => Ok(value),\n        Err(error) => Err(error.into()),\n    }\n}\n\nfn skip(row: Row) {\n    if let Err(error) = save(row) {\n        warn(&error);\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoSilentErrorArm, text), NONE);
    }

    #[test]
    fn dropping_failed_items_is_reported() {
        let text = "fn names(paths: &[Path]) -> Vec<String> {\n    let read: Vec<_> = paths.iter().map(read).filter_map(Result::ok).collect();\n    let parsed: Vec<u8> = TEXT.split(',').filter_map(|part| part.parse().ok()).collect();\n    let entries: Vec<_> = std::fs::read_dir(ROOT)?.flatten().collect();\n    let nested: Vec<u8> = groups.into_iter().flatten().collect();\n    let present: Vec<u8> = maybe.iter().filter_map(|value| value.as_ref()).collect();\n}\n";

        assert_eq!(lines_of(Rule::NoFilterMapOk, text), [2, 3, 4]);
    }

    #[test]
    fn discarding_assignments_are_reported() {
        let text = "fn save(file: File, unused: u8) {\n    _ = file.sync_all();\n    let _ = unused;\n    let _ = file.flush();\n}\n";

        assert_eq!(lines_of(Rule::NoDiscardAssign, text), [2, 3]);
    }

    #[test]
    fn tests_may_drop_errors() {
        let text = "fn cleanup() {\n    _ = std::fs::remove_dir_all(TEMP);\n}\n";

        assert_eq!(
            lines_in(Rule::NoDiscardAssign, "tests/cleanup.rs", text),
            NONE
        );
    }
}
