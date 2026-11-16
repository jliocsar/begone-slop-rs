//! `no-anyhow-in-lib`: a library's errors are types its callers can match on.
//!
//! `anyhow::Error` and `eyre::Report` erase what went wrong into a message, which is right
//! for a binary that only prints it and wrong for a library whose callers have to decide what
//! a failure means. Binaries, build scripts, tests, examples and benches may use them.

use syn::ItemUse;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use super::paths::{segments, use_leaves};
use super::{FileContext, Finding, Role, Rule};

const OPAQUE_ERROR_CRATES: [&str; 3] = ["anyhow", "color_eyre", "eyre"];

struct LibraryErrors<'run> {
    file: &'run FileContext<'run>,
    findings: Vec<Finding>,
}

impl LibraryErrors<'_> {
    fn report(&mut self, line: usize, full: &[String]) {
        let Some(root) = full.first() else {
            return;
        };

        if self.file.is_test(line) || !OPAQUE_ERROR_CRATES.contains(&root.as_str()) {
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

impl<'ast> Visit<'ast> for LibraryErrors<'_> {
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
            .is_some_and(|first| self.file.imports.contains(first));

        if path.leading_colon.is_some() || !imported {
            self.report(path.span().start().line, &written);
        }

        visit::visit_path(self, path);
    }
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    if file.role != Role::Library {
        return Vec::new();
    }

    let mut errors = LibraryErrors {
        file,
        findings: Vec::new(),
    };

    errors.visit_file(file.syntax);
    errors
        .findings
        .dedup_by_key(|finding| finding.line);

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
