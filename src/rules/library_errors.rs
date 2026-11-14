//! `no-anyhow-in-lib`: a library's errors are types its callers can match on.
//!
//! `anyhow::Error` and `eyre::Report` erase what went wrong into a message, which is right
//! for a binary that only prints it and wrong for a library whose callers have to decide what
//! a failure means. Binaries, build scripts, tests, examples and benches may use them.

use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{ItemFn, ItemMod, ItemUse};

use super::paths::{file_imports, segments, use_leaves};
use super::test_scope::{is_test_fn, is_test_module};
use super::{Context, Finding, Role, Rule};

const OPAQUE_ERROR_CRATES: [&str; 3] = ["anyhow", "color_eyre", "eyre"];

struct LibraryErrors {
    imports: HashMap<String, Vec<String>>,
    findings: Vec<Finding>,
    test_depth: usize,
}

impl LibraryErrors {
    fn report(&mut self, line: usize, full: &[String]) {
        let Some(root) = full.first() else {
            return;
        };

        if self.test_depth > 0 || !OPAQUE_ERROR_CRATES.contains(&root.as_str()) {
            return;
        }

        self.findings.push(Finding {
            line,
            rule: Rule::NoAnyhowInLib,
            message: format!("`{root}` in library code erases the error's type, so callers can only print it; define an error enum (`thiserror`) and leave `{root}` to the binary"),
            edit: None,
        });
    }
}

impl<'ast> Visit<'ast> for LibraryErrors {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let is_test = is_test_fn(&function.attrs);
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

    fn visit_item_use(&mut self, import: &'ast ItemUse) {
        for leaf in use_leaves(&import.tree) {
            self.report(leaf.line, &leaf.full);
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let written = segments(path);
        // A name brought in by a `use` was reported there.
        let imported = written
            .first()
            .is_some_and(|first| self.imports.contains_key(first));

        if path.leading_colon.is_some() || !imported {
            self.report(path.span().start().line, &written);
        }

        visit::visit_path(self, path);
    }
}

pub(super) fn check(file: &syn::File, context: &Context) -> Vec<Finding> {
    if context.role != Role::Library {
        return Vec::new();
    }

    let mut errors = LibraryErrors {
        imports: file_imports(file),
        findings: Vec::new(),
        test_depth: 0,
    };
    errors.visit_file(file);
    errors.findings.dedup_by_key(|finding| finding.line);

    errors.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_in, lines_of};

    #[test]
    fn anyhow_in_a_library_is_reported() {
        let text = "use anyhow::Context;\n\nfn load() -> anyhow::Result<Config> {\n    eyre::bail!(\"no config\");\n}\n";

        assert_eq!(lines_of(Rule::NoAnyhowInLib, text), [1, 3, 4]);
    }

    #[test]
    fn binaries_and_tests_may_use_it() {
        let text = "fn main() -> anyhow::Result<()> {\n    run()\n}\n";

        assert_eq!(lines_in(Rule::NoAnyhowInLib, "src/main.rs", text), NONE);
        assert_eq!(lines_in(Rule::NoAnyhowInLib, "tests/load.rs", text), NONE);
    }
}
