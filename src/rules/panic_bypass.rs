//! `no-panic-bypass`: shipped code doesn't panic through the door the preset left open.
//!
//! The preset bans `panic!`, `todo!`, `unimplemented!`, `unwrap` and `expect`, plus (through
//! clippy's `unreachable` and `exit`) `unreachable!` and `process::exit` outside `main`. What's
//! left is `assert!`, `assert_eq!` and `assert_ne!`, which panic just the same, and which
//! clippy's own `manual_assert` turns an `if c { panic!() }` into. So outside tests,
//! examples, benches and build scripts they're findings, except in a `const` context, where a
//! failed assertion stops the build instead. `debug_assert!` stays: it checks an invariant
//! in tests and costs nothing in release.

use syn::visit::{self, Visit};
use syn::{ExprConst, ImplItemConst, ItemConst, ItemFn, ItemMod, ItemStatic, Macro};

use super::test_scope::{is_test_fn, is_test_module};
use super::{Context, Finding, Role, Rule};

const PANICKING_ASSERTS: [&str; 3] = ["assert", "assert_eq", "assert_ne"];

struct PanicBypass {
    findings: Vec<Finding>,
    /// Inside test code or a `const` context, where an assertion is fine.
    exempt_depth: usize,
}

impl PanicBypass {
    fn exempt(&mut self, walk: impl FnOnce(&mut Self)) {
        self.exempt_depth += 1;
        walk(self);
        self.exempt_depth -= 1;
    }
}

impl<'ast> Visit<'ast> for PanicBypass {
    fn visit_expr_const(&mut self, block: &'ast ExprConst) {
        self.exempt(|check| visit::visit_expr_const(check, block));
    }

    fn visit_impl_item_const(&mut self, constant: &'ast ImplItemConst) {
        self.exempt(|check| visit::visit_impl_item_const(check, constant));
    }

    fn visit_item_const(&mut self, constant: &'ast ItemConst) {
        self.exempt(|check| visit::visit_item_const(check, constant));
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        if is_test_fn(&function.attrs) {
            self.exempt(|check| visit::visit_item_fn(check, function));
        } else {
            visit::visit_item_fn(self, function);
        }
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if is_test_module(&module.attrs) {
            self.exempt(|check| visit::visit_item_mod(check, module));
        } else {
            visit::visit_item_mod(self, module);
        }
    }

    fn visit_item_static(&mut self, item: &'ast ItemStatic) {
        self.exempt(|check| visit::visit_item_static(check, item));
    }

    fn visit_macro(&mut self, invocation: &'ast Macro) {
        let name = invocation
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();

        if self.exempt_depth == 0 && PANICKING_ASSERTS.contains(&name.as_str()) {
            self.findings.push(Finding {
                line: invocation.path.segments.first().map_or(0, |segment| segment.ident.span().start().line),
                rule: Rule::NoPanicBypass,
                message: format!("`{name}!` panics in shipped code like the `panic!` the preset bans; return an error, or use `debug_assert!` for an invariant"),
                edit: None,
            });
        }

        visit::visit_macro(self, invocation);
    }
}

pub(super) fn check(file: &syn::File, context: &Context) -> Vec<Finding> {
    if !matches!(context.role, Role::Library | Role::Binary) {
        return Vec::new();
    }

    let mut bypass = PanicBypass {
        findings: Vec::new(),
        exempt_depth: 0,
    };
    bypass.visit_file(file);

    bypass.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_in, lines_of};

    #[test]
    fn asserts_in_shipped_code_are_reported() {
        let text = "fn resize(width: u16) {\n    assert!(width > 0, \"width\");\n    assert_eq!(width % 2, 0);\n    debug_assert!(width < MAX);\n}\n";

        assert_eq!(lines_of(Rule::NoPanicBypass, text), [2, 3]);
    }

    #[test]
    fn const_assertions_and_tests_pass() {
        let text = "const _: () = assert!(SIZE > 0);\n\nfn sized() {\n    const { assert!(SIZE < 64) };\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn sizes_match() {\n        assert_eq!(SIZE, 8);\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoPanicBypass, text), NONE);
    }

    #[test]
    fn examples_may_assert() {
        let text = "fn main() {\n    assert!(run());\n}\n";

        assert_eq!(
            lines_in(Rule::NoPanicBypass, "examples/demo.rs", text),
            NONE
        );
    }
}
