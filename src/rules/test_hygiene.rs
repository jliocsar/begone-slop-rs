//! `test-name` and `no-sleep-in-tests`: tests that say what they prove and don't race the clock.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, File, ItemFn, ItemMod};

use super::test_scope::{is_test_fn, is_test_module, is_test_path};
use super::{Finding, Rule};
use crate::source::Source;

/// Names that say a test exists but not what it proves.
const VAGUE_TEST_NAMES: [&str; 4] = ["it_works", "works", "basic", "smoke"];

struct TestHygiene {
    findings: Vec<Finding>,
    test_file: bool,
    test_depth: usize,
}

impl TestHygiene {
    fn in_tests(&self) -> bool {
        self.test_file || self.test_depth > 0
    }

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
}

impl<'ast> Visit<'ast> for TestHygiene {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        let sleeps = matches!(&*call.func, Expr::Path(callee)
            if callee.path.segments.last().is_some_and(|segment| segment.ident == "sleep"));

        if self.in_tests() && sleeps {
            self.findings.push(Finding {
                line: call.func.span().start().line,
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
        }

        self.test_depth += usize::from(is_test);
        visit::visit_item_fn(self, function);
        self.test_depth -= usize::from(is_test);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        let is_test = is_test_module(&module.attrs);
        self.test_depth += usize::from(is_test);
        visit::visit_item_mod(self, module);
        self.test_depth -= usize::from(is_test);
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

pub(super) fn check(source: &Source, file: &File) -> Vec<Finding> {
    let mut hygiene = TestHygiene {
        findings: Vec::new(),
        test_file: is_test_path(&source.path),
        test_depth: 0,
    };
    hygiene.visit_file(file);

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
