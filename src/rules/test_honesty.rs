//! `test-conditional-assert`, `test-early-return` and `no-cfg-test-in-prod`: a test that passes
//! means the shipped code works.
//!
//! An assertion inside `if let Ok(..) = ..` with no `else` (or a `match` whose other arm is
//! `_ => {}`) never runs when the precondition fails, and the test passes. So does a test that
//! returns early (`if !server_up() { return; }`): it's a skip that reports success, which is
//! what `#[ignore = "why"]` is for. And code that checks `cfg!(test)`, carries a
//! `#[cfg(test)]` statement inside a shipped function, or ships a `*_for_test` function
//! behaves one way under test and another in production.

use std::collections::BTreeSet;
use std::slice;

use proc_macro2::{TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Arm, Attribute, Block, Expr, ExprClosure, ExprIf, ExprMatch, ExprReturn, Ident, ImplItemFn,
    ItemFn, ItemImpl, ItemMod, Local, Macro, Pat, TraitItemFn,
};

use super::test_scope::{is_test_fn, is_test_module};
use super::{Context, Finding, Role, Rule};
use crate::module_paths::is_test_only;

const ASSERTING_PREFIXES: [&str; 3] = ["assert", "debug_assert", "prop_assert"];
const TEST_SUFFIXES: [&str; 3] = ["_for_test", "_for_testing", "_for_tests"];
const TEST_PREFIX: &str = "test_only_";

/// Walks one `#[test]` body: where its assertions sit, and where it returns.
#[derive(Default)]
struct TestBody {
    findings: Vec<Finding>,
    /// Inside a branch that may not run: an `if let` with no `else`, or a `match` arm next to
    /// `_ => {}`. Holds the line of the outermost one.
    conditional: Option<usize>,
    /// Inside any branch at all, where a `return` skips the rest.
    branch_depth: usize,
    /// The branches that may not run and hold assertions, by line.
    skippable: BTreeSet<usize>,
}

/// Walks shipped code for test-only switches.
struct Production {
    findings: Vec<Finding>,
    fn_depth: usize,
}

/// Every `#[test]` function in the file.
struct Tests {
    findings: Vec<Finding>,
}

impl TestBody {
    fn of(function: &ItemFn) -> Vec<Finding> {
        let mut body = Self::default();
        body.visit_block(&function.block);

        let skippable = std::mem::take(&mut body.skippable);
        body.findings.extend(skippable.into_iter().map(|line| Finding {
            line,
            rule: Rule::TestConditionalAssert,
            message: "the assertions here only run when this branch does, so the test passes when it doesn't; unwrap the value (or assert the case) first".to_owned(),
            edit: None,
        }));

        body.findings
    }

    fn branch(&mut self, conditional: Option<usize>, walk: impl FnOnce(&mut Self)) {
        let outer = self.conditional;
        self.conditional = outer.or(conditional);
        self.branch_depth += 1;
        walk(self);
        self.branch_depth -= 1;
        self.conditional = outer;
    }
}

impl<'ast> Visit<'ast> for TestBody {
    fn visit_expr_closure(&mut self, _closure: &'ast ExprClosure) {}

    fn visit_expr_if(&mut self, branch: &'ast ExprIf) {
        self.visit_expr(&branch.cond);
        let may_skip = matches!(&*branch.cond, Expr::Let(_)) && branch.else_branch.is_none();
        let line = may_skip.then(|| branch.if_token.span.start().line);

        self.branch(line, |body| body.visit_block(&branch.then_branch));

        if let Some((_, otherwise)) = &branch.else_branch {
            self.branch(None, |body| body.visit_expr(otherwise));
        }
    }

    fn visit_expr_match(&mut self, matching: &'ast ExprMatch) {
        self.visit_expr(&matching.expr);
        let skips = matching.arms.iter().any(is_skipping_arm);
        let line = skips.then(|| matching.match_token.span.start().line);

        for arm in &matching.arms {
            self.branch(line, |body| body.visit_arm(arm));
        }
    }

    fn visit_expr_return(&mut self, stop: &'ast ExprReturn) {
        let passes = stop.expr.as_deref().is_none_or(is_unit_ok);

        if self.branch_depth > 0 && passes {
            self.findings.push(Finding {
                line: stop.return_token.span.start().line,
                rule: Rule::TestEarlyReturn,
                message: "returning early skips the test and still reports a pass; fail instead, or mark it `#[ignore = \"why\"]`".to_owned(),
                edit: None,
            });
        }

        visit::visit_expr_return(self, stop);
    }

    fn visit_item(&mut self, _nested: &'ast syn::Item) {}

    fn visit_local(&mut self, local: &'ast Local) {
        for attribute in &local.attrs {
            self.visit_attribute(attribute);
        }

        self.visit_pat(&local.pat);

        if let Some(init) = &local.init {
            self.visit_expr(&init.expr);

            if let Some((_, otherwise)) = &init.diverge {
                self.branch(None, |body| body.visit_expr(otherwise));
            }
        }
    }

    fn visit_macro(&mut self, invocation: &'ast Macro) {
        let name = invocation
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();
        let asserts = ASSERTING_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix));

        if asserts && let Some(line) = self.conditional {
            self.skippable.insert(line);
        }
    }
}

impl Production {
    fn push(&mut self, line: usize, message: String) {
        self.findings.push(Finding {
            line,
            rule: Rule::NoTestSwitchesInProduction,
            message,
            edit: None,
        });
    }

    fn name(&mut self, ident: &Ident) {
        let name = ident.to_string();
        let for_tests = TEST_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
            || name.starts_with(TEST_PREFIX);

        if for_tests {
            self.push(
                ident.span().start().line,
                format!("`{name}` ships a test-only path in production code; move it under `#[cfg(test)]`, or test through the real API"),
            );
        }
    }

    /// A shipped function: its name, then its body.
    fn function(&mut self, attributes: &[Attribute], ident: &Ident, block: &Block) {
        if self.fn_depth > 0 {
            for attribute in attributes {
                self.visit_attribute(attribute);
            }
        }

        if is_test_fn(attributes) || is_test_only(attributes) {
            return;
        }

        self.name(ident);
        self.fn_depth += 1;
        self.visit_block(block);
        self.fn_depth -= 1;
    }
}

impl<'ast> Visit<'ast> for Production {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if self.fn_depth > 0 && is_test_only(slice::from_ref(attribute)) {
            self.push(
                attribute.pound_token.span.start().line,
                "`#[cfg(test)]` inside a shipped function makes the tested code differ from the shipped code; pass the difference in instead".to_owned(),
            );
        }
    }

    fn visit_impl_item_fn(&mut self, method: &'ast ImplItemFn) {
        self.function(&method.attrs, &method.sig.ident, &method.block);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.function(&function.attrs, &function.sig.ident, &function.block);
    }

    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        if !is_test_only(&implementation.attrs) {
            visit::visit_item_impl(self, implementation);
        }
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if !is_test_module(&module.attrs) {
            visit::visit_item_mod(self, module);
        }
    }

    fn visit_macro(&mut self, invocation: &'ast Macro) {
        if invocation.path.is_ident("cfg") && mentions_test(invocation.tokens.clone()) {
            self.push(
                invocation.path.span().start().line,
                "`cfg!(test)` makes the tested code differ from the shipped code; pass the difference in instead".to_owned(),
            );
        }

        visit::visit_macro(self, invocation);
    }

    fn visit_trait_item_fn(&mut self, method: &'ast TraitItemFn) {
        if let Some(block) = &method.default {
            self.function(&method.attrs, &method.sig.ident, block);
        } else {
            self.name(&method.sig.ident);
        }
    }
}

impl<'ast> Visit<'ast> for Tests {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        if is_test_fn(&function.attrs) {
            self.findings.extend(TestBody::of(function));
        }

        visit::visit_item_fn(self, function);
    }
}

/// Whether a `cfg` predicate names `test` anywhere: `test`, `any(test, feature = "x")`.
fn mentions_test(tokens: TokenStream) -> bool {
    tokens.into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == "test",
        TokenTree::Group(group) => mentions_test(group.stream()),
        TokenTree::Punct(_) | TokenTree::Literal(_) => false,
    })
}

/// `_ => {}`, `_ => ()`, `Err(_) => {}`, `None => {}`: an arm where the rest is skipped.
fn is_skipping_arm(arm: &Arm) -> bool {
    let catch_all = matches!(&arm.pat, Pat::Wild(_))
        || matches!(&arm.pat, Pat::Ident(binding) if binding.ident == "None")
        || matches!(&arm.pat, Pat::Path(path) if path.path.is_ident("None"))
        || matches!(&arm.pat, Pat::TupleStruct(tuple) if tuple.path.is_ident("Err"));

    let empty = match &*arm.body {
        Expr::Block(block) => block.block.stmts.is_empty(),
        Expr::Tuple(tuple) => tuple.elems.is_empty(),
        _ => false,
    };

    catch_all && empty
}

/// `Ok(())`, which passes a test that returns a `Result`.
fn is_unit_ok(value: &Expr) -> bool {
    matches!(value, Expr::Call(call)
        if matches!(&*call.func, Expr::Path(path) if path.path.is_ident("Ok"))
            && call.args.len() == 1
            && call.args.first().is_some_and(|argument| matches!(argument, Expr::Tuple(unit) if unit.elems.is_empty())))
}

pub(super) fn check(file: &syn::File, context: &Context) -> Vec<Finding> {
    let mut tests = Tests {
        findings: Vec::new(),
    };
    tests.visit_file(file);
    let mut findings = tests.findings;

    if matches!(context.role, Role::Library | Role::Binary) {
        let mut production = Production {
            findings: Vec::new(),
            fn_depth: 0,
        };
        production.visit_file(file);
        findings.extend(production.findings);
    }

    findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn an_assertion_under_if_let_is_reported() {
        let text = "#[test]\nfn parses_the_header() {\n    if let Ok(header) = parse(INPUT) {\n        assert_eq!(header.len, 3);\n    }\n}\n\n#[test]\nfn reads_the_kind() {\n    match parse(INPUT) {\n        Ok(header) => assert_eq!(header.kind, Kind::Data),\n        _ => {}\n    }\n}\n\n#[test]\nfn checks_the_case_then_the_length() {\n    let header = parse(INPUT);\n\n    assert!(header.is_ok());\n\n    if let Ok(header) = header {\n        assert_eq!(header.len, 3);\n    }\n}\n";

        assert_eq!(lines_of(Rule::TestConditionalAssert, text), [3, 10, 22]);
    }

    #[test]
    fn an_early_return_in_a_test_is_reported() {
        let text = "#[test]\nfn reconnects() {\n    if !server_running() {\n        return;\n    }\n\n    let Some(socket) = connect() else {\n        return;\n    };\n\n    let check = || {\n        if ready() {\n            return;\n        }\n    };\n\n    assert!(socket.is_open());\n}\n";

        assert_eq!(lines_of(Rule::TestEarlyReturn, text), [4, 8]);
    }

    #[test]
    fn shipped_code_with_test_switches_is_reported() {
        let text = "fn connect() -> Client {\n    if cfg!(test) {\n        return fake();\n    }\n\n    #[cfg(test)]\n    let client = fake();\n\n    real()\n}\n\npub fn reset_for_tests() {}\n\n#[cfg(test)]\nimpl Client {\n    fn new_for_test() -> Self {\n        fake()\n    }\n}\n\n#[cfg(test)]\nmod tests {\n    fn helper_for_tests() {}\n}\n";

        assert_eq!(lines_of(Rule::NoTestSwitchesInProduction, text), [2, 6, 12]);
    }

    #[test]
    fn honest_tests_pass() {
        let text = "#[test]\nfn parses_the_header() {\n    let header = parse(INPUT).unwrap();\n\n    assert_eq!(header.len, 3);\n}\n";

        assert_eq!(lines_of(Rule::TestConditionalAssert, text), NONE);
        assert_eq!(lines_of(Rule::TestEarlyReturn, text), NONE);
    }
}
