//! What counts as test code inside a file: `#[test]`-like functions and `#[cfg(test)]`
//! modules. Whole files are test code by their [`Role`].

use std::ops::RangeInclusive;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, File, ItemFn, ItemMod};

use super::Role;
use crate::module_paths;

/// The lines of one file that are test code, worked out once so no rule tracks it again.
#[derive(Debug, Default)]
pub(super) struct TestLines {
    whole_file: bool,
    ranges: Vec<RangeInclusive<usize>>,
}

impl TestLines {
    /// Every line of a test file, else the lines of each test function and test module,
    /// from its first attribute to its closing brace.
    pub(super) fn of(file: &File, role: Role) -> Self {
        let mut lines = Self {
            whole_file: role.is_test(),
            ranges: Vec::new(),
        };

        if !lines.whole_file {
            lines.visit_file(file);
        }

        lines
    }

    /// Whether `line` is test code.
    pub(super) fn contains(&self, line: usize) -> bool {
        self.whole_file || self.ranges.iter().any(|range| range.contains(&line))
    }
}

impl<'ast> Visit<'ast> for TestLines {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        if is_test_fn(&function.attrs) {
            let last = function.block.brace_token.span.close().end().line;
            self.ranges.push(function.span().start().line..=last);
        } else {
            visit::visit_item_fn(self, function);
        }
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if !is_test_module(&module.attrs) {
            visit::visit_item_mod(self, module);

            return;
        }

        let last = match (&module.content, &module.semi) {
            (Some((brace, _)), _) => brace.span.close().end().line,
            (None, Some(semi)) => semi.span.end().line,
            (None, None) => module.ident.span().end().line,
        };

        self.ranges.push(module.span().start().line..=last);
    }
}

/// Any attribute whose last segment is `test`: `#[test]`, `#[tokio::test]`, `#[rstest::test]`.
pub(super) fn is_test_fn(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "test")
    })
}

/// `#[cfg(test)]`, or `test` inside `cfg(all(...))`.
pub(super) fn is_test_module(attributes: &[Attribute]) -> bool {
    module_paths::is_test_only(attributes)
}

#[cfg(test)]
mod tests {
    use super::TestLines;
    use crate::rules::Role;

    fn test_lines(text: &str) -> Vec<usize> {
        let file = syn::parse_file(text).unwrap();
        let lines = TestLines::of(&file, Role::Library);

        (1..=text.lines().count())
            .filter(|line| lines.contains(*line))
            .collect()
    }

    #[test]
    fn attributes_and_body_of_a_test_item_are_test_code() {
        let text = "fn shipped() {}\n\n#[test]\nfn checks() {\n    run();\n}\n\n#[cfg(test)]\nmod tests {\n    fn helper() {}\n}\n";

        assert_eq!(test_lines(text), [3, 4, 5, 6, 8, 9, 10, 11]);
    }

    #[test]
    fn a_test_file_is_test_code_throughout() {
        let file = syn::parse_file("fn helper() {}\n").unwrap();

        assert!(TestLines::of(&file, Role::Test).contains(1));
    }
}
