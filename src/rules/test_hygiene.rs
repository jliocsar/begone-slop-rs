//! `test-name`, `no-sleep-in-tests` and `test-must-assert`: tests that say what they prove,
//! prove it, and don't race the clock.
//!
//! A test asserts when its body has an assertion macro (`assert*!`, `debug_assert*!`,
//! `prop_assert*!`, `insta::assert_*!`, `panic!`, `unreachable!`) or an assertion method
//! (`.assert()`, as `assert_cmd` has it), is `#[should_panic]`, or calls a function in the
//! same file that asserts. An assertion of constants (`assert!(true)`, `assert_eq!(1, 1)`)
//! checks nothing, so it doesn't count. Running code without checking its result proves only
//! that it didn't panic.

use std::collections::{HashMap, HashSet};

use quote::ToTokens;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Block, Expr, ExprCall, ExprMethodCall, ImplItemFn, ItemFn, Macro, UnOp};

use super::paths::macro_arguments;
use super::test_scope::{TestLines, is_test_fn};
use super::{FileContext, Finding, Rule};

/// Names that say a test exists but not what it proves.
const VAGUE_TEST_NAMES: [&str; 4] = ["it_works", "works", "basic", "smoke"];

const PANICKING_MACROS: [&str; 2] = ["panic", "unreachable"];
const ASSERTING_PREFIXES: [&str; 3] = ["assert", "debug_assert", "prop_assert"];

struct TestHygiene<'run> {
    tests: &'run TestLines,
    findings: Vec<Finding>,
    asserting: HashSet<String>,
}

/// What one function body asserts directly, and which functions it calls.
#[derive(Default)]
struct Body {
    asserts: bool,
    calls: HashSet<String>,
}

/// Every function in the file, by name, with its [`Body`].
#[derive(Default)]
struct Bodies {
    bodies: HashMap<String, Body>,
}

impl TestHygiene<'_> {
    fn name_test(&mut self, function: &ItemFn) {
        let name = function.sig.ident.to_string();

        if !is_vague(&name) {
            return;
        }

        self.findings.push(Finding {
            line: function.sig.ident.span().start().line,
            rule: Rule::TestName,
            message: format!(
                "`{name}` doesn't say what the test proves; name it after the behaviour, like `resume_counts_only_consumed_bytes`"
            ),
            edit: None,
        });
    }

    fn require_assertion(&mut self, function: &ItemFn) {
        let should_panic = function
            .attrs
            .iter()
            .any(|attribute| attribute.path().is_ident("should_panic"));
        let body = Body::of(&function.block);
        let asserts = body.asserts || body.calls.iter().any(|name| self.asserting.contains(name));

        if !should_panic && !asserts {
            self.findings.push(Finding {
                line: function.sig.ident.span().start().line,
                rule: Rule::TestMustAssert,
                message: format!(
                    "`{}` runs code but never checks a result; assert what it should have done",
                    function.sig.ident
                ),
                edit: None,
            });
        }
    }
}

impl Body {
    fn of(block: &Block) -> Self {
        let mut body = Self::default();

        body.visit_block(block);

        body
    }
}

impl<'ast> Visit<'ast> for Body {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(callee) = &*call.func
            && let Some(last) = callee.path.segments.last()
        {
            self.calls.insert(last.ident.to_string());
        }

        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        let method = call.method.to_string();

        self.asserts |= method.starts_with("assert");
        self.calls.insert(method);
        visit::visit_expr_method_call(self, call);
    }

    fn visit_macro(&mut self, invocation: &'ast Macro) {
        let name = invocation
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();
        let asserting = ASSERTING_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix));

        self.asserts |= PANICKING_MACROS.contains(&name.as_str())
            || (asserting && !asserts_constants(&name, invocation));
    }
}

impl Bodies {
    /// Functions that assert, directly or through other functions in the file.
    fn asserting(&self) -> HashSet<String> {
        let mut asserting: HashSet<String> = self
            .bodies
            .iter()
            .filter(|(_, body)| body.asserts)
            .map(|(name, _)| name.clone())
            .collect();

        loop {
            let reached: Vec<String> = self
                .bodies
                .iter()
                .filter(|(name, body)| {
                    !asserting.contains(*name)
                        && body.calls.iter().any(|call| asserting.contains(call))
                })
                .map(|(name, _)| name.clone())
                .collect();

            if reached.is_empty() {
                return asserting;
            }

            asserting.extend(reached);
        }
    }
}

impl<'ast> Visit<'ast> for Bodies {
    fn visit_impl_item_fn(&mut self, method: &'ast ImplItemFn) {
        self.bodies
            .insert(method.sig.ident.to_string(), Body::of(&method.block));
        visit::visit_impl_item_fn(self, method);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.bodies
            .insert(function.sig.ident.to_string(), Body::of(&function.block));
        visit::visit_item_fn(self, function);
    }
}

impl<'ast> Visit<'ast> for TestHygiene<'_> {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        let sleeps = matches!(&*call.func, Expr::Path(callee)
            if callee.path.segments.last().is_some_and(|segment| segment.ident == "sleep"));
        let line = call.func.span().start().line;

        if self.tests.contains(line) && sleeps {
            self.findings.push(Finding {
                line,
                rule: Rule::NoSleepInTests,
                message: "a sleep in a test is a race with the machine's load; wait on the event itself (a channel, a condition, a timeout around the real signal)".to_owned(),
                edit: None,
            });
        }

        visit::visit_expr_call(self, call);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let is_test = is_test_fn(&function.attrs);

        if is_test {
            self.name_test(function);
            self.require_assertion(function);
        }

        visit::visit_item_fn(self, function);
    }
}

/// `assert!(true)`, `assert!(1 == 1)`, `assert_eq!(2, 2)`, `assert_eq!(x, x)`: the outcome is
/// known before the test runs.
fn asserts_constants(name: &str, invocation: &Macro) -> bool {
    let arguments = macro_arguments(invocation);
    let compares = name.ends_with("_eq") || name.ends_with("_ne");

    match arguments.as_slice() {
        [left, right, ..] if compares => {
            let same = left.to_token_stream().to_string() == right.to_token_stream().to_string();

            same || (is_constant(left) && is_constant(right))
        }
        [condition, ..] if !compares && !name.contains('_') => is_constant(condition),
        _ => false,
    }
}

/// A literal, or an expression built only from literals.
fn is_constant(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(_) => true,
        Expr::Paren(inner) => is_constant(&inner.expr),
        Expr::Unary(unary) => {
            matches!(unary.op, UnOp::Not(_) | UnOp::Neg(_)) && is_constant(&unary.expr)
        }
        Expr::Binary(binary) => is_constant(&binary.left) && is_constant(&binary.right),
        _ => false,
    }
}

fn is_vague(name: &str) -> bool {
    let bare = name.trim_start_matches('_');

    VAGUE_TEST_NAMES.contains(&bare)
        || bare == "test"
        || bare.starts_with("test_")
        || bare.ends_with("_test")
        || bare
            .strip_prefix("test")
            .is_some_and(|rest| rest.chars().all(|character| character.is_ascii_digit()))
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let syntax = file.syntax;
    let mut bodies = Bodies::default();

    bodies.visit_file(syntax);

    let mut hygiene = TestHygiene {
        tests: &file.tests,
        findings: Vec::new(),
        asserting: bodies.asserting(),
    };

    hygiene.visit_file(syntax);

    hygiene.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn a_test_named_test_something_is_reported() {
        let text =
            "#[test]\nfn test_parse() {}\n\n#[test]\nfn it_works() {}\n\n#[test]\nfn test2() {}\n";

        assert_eq!(lines_of(Rule::TestName, text), [2, 5, 8]);
    }

    #[test]
    fn a_behaviour_name_passes() {
        let text = "#[tokio::test]\nasync fn resume_counts_only_consumed_bytes() {}\n";

        assert_eq!(lines_of(Rule::TestName, text), NONE);
    }

    #[test]
    fn helpers_outside_tests_may_be_named_anything() {
        let text = "fn test_fixture() {}\n";

        assert_eq!(lines_of(Rule::TestName, text), NONE);
    }

    #[test]
    fn a_test_that_never_checks_is_reported() {
        let text = "#[test]\nfn parses_a_frame() {\n    let frame = parse(BYTES);\n}\n";

        assert_eq!(lines_of(Rule::TestMustAssert, text), [2]);
    }

    #[test]
    fn asserting_through_a_helper_counts() {
        let text = "fn round_trips(input: &str) {\n    let output = render(&parse(input));\n\n    assert_eq!(output, input);\n}\n\nfn check(input: &str) {\n    round_trips(input);\n}\n\n#[test]\nfn frames_round_trip() {\n    check(\"a\");\n}\n\n#[test]\n#[should_panic(expected = \"empty\")]\nfn an_empty_frame_panics() {\n    parse(\"\");\n}\n";

        assert_eq!(lines_of(Rule::TestMustAssert, text), NONE);
    }

    #[test]
    fn asserting_constants_doesnt_count() {
        let text = "#[test]\nfn always_passes() {\n    run();\n    assert!(true);\n    assert_eq!(1 + 1, 2);\n    assert_ne!(SIZE, SIZE);\n}\n\n#[test]\nfn checks_the_output() {\n    let output = run();\n\n    assert!(output.is_empty(), \"{output}\");\n}\n";

        assert_eq!(lines_of(Rule::TestMustAssert, text), [2]);
    }

    #[test]
    fn assertion_methods_and_snapshot_macros_count() {
        let text = "#[test]\nfn exits_cleanly() {\n    Command::cargo_bin(NAME).assert().success();\n}\n\n#[test]\nfn renders_the_frame() {\n    insta::assert_snapshot!(render(FRAME));\n}\n";

        assert_eq!(lines_of(Rule::TestMustAssert, text), NONE);
    }

    #[test]
    fn sleeping_in_a_test_is_reported() {
        let text = "#[test]\nfn reconnects() {\n    std::thread::sleep(WAIT);\n}\n";

        assert_eq!(lines_of(Rule::NoSleepInTests, text), [3]);
    }

    #[test]
    fn sleeping_in_a_test_module_helper_is_reported() {
        let text = "#[cfg(test)]\nmod tests {\n    async fn settle() {\n        tokio::time::sleep(WAIT).await;\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoSleepInTests, text), [4]);
    }

    #[test]
    fn sleeping_outside_tests_is_left_alone() {
        let text = "fn backoff() {\n    std::thread::sleep(WAIT);\n}\n";

        assert_eq!(lines_of(Rule::NoSleepInTests, text), NONE);
    }
}
