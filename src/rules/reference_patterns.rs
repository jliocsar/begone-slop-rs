//! `no-reference-patterns`: take a value at its source, not with `&` in a binding.
//!
//! `.filter(|&n| n % 2 != 0)` and `let &width = widths.first()?;` copy a value out of a
//! reference with a pattern, which a reader has to stop and decode. Copy it where it comes from
//! (`.copied()`), or keep the reference: operators and method calls see through it
//! (`number % 2 != 0` works on a `&i32`), and `*` reads it where a value is needed. Checked in
//! closure parameters and `let` statements, on an `&` or `&mut` right around a binding,
//! anywhere in the pattern (`|(index, &value)|`). An `&` that destructures
//! (`|&(path, text)|`, `&[first, ..]`, `&Point { x, y }`) passes: it unpacks a borrowed tuple
//! or struct in one step, where the alternative is a `*` at every use. `if let` and `match`
//! patterns are left alone, and `&ref x` is clippy's `needless_borrowed_reference`.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{ExprClosure, Local, Pat, PatIdent, PatReference};

use super::{FileContext, Finding, Rule};

struct ReferencePatterns {
    findings: Vec<Finding>,
    in_binding: bool,
}

impl<'ast> Visit<'ast> for ReferencePatterns {
    fn visit_expr_closure(&mut self, closure: &'ast ExprClosure) {
        for input in &closure.inputs {
            self.in_binding = true;
            self.visit_pat(input);
            self.in_binding = false;
        }

        self.visit_return_type(&closure.output);
        self.visit_expr(&closure.body);
    }

    fn visit_local(&mut self, binding: &'ast Local) {
        self.in_binding = true;
        self.visit_pat(&binding.pat);
        self.in_binding = false;

        if let Some(init) = &binding.init {
            self.visit_local_init(init);
        }
    }

    fn visit_pat_reference(&mut self, reference: &'ast PatReference) {
        if self.in_binding
            && let Pat::Ident(PatIdent {
                by_ref: None,
                ident,
                ..
            }) = &*reference.pat
        {
            self.findings.push(Finding {
                line: reference.span().start().line,
                rule: Rule::NoReferencePatterns,
                message: format!("`&{ident}` copies the value out with a pattern; copy it at the source (`.copied()`), or keep the reference and write `*{ident}` where a value is needed"),
                edit: None,
            });
        }

        visit::visit_pat_reference(self, reference);
    }
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut patterns = ReferencePatterns {
        findings: Vec::new(),
        in_binding: false,
    };

    patterns.visit_file(file.syntax);

    patterns.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn a_reference_around_a_binding_is_reported() {
        let text = "fn main() {\n    let odd = numbers.iter().filter(|&number| number % 2 != 0);\n    let item = |&index: &usize| &rows[index];\n    let indexed = numbers.iter().enumerate().map(|(index, &number)| index + number);\n    let &width = widths.first()?;\n    let Some(&height) = heights.first() else { return };\n    let bumped = cells.iter_mut().map(|&mut cell| cell + 1);\n}\n";

        assert_eq!(
            lines_of(Rule::NoReferencePatterns, text),
            [2, 3, 4, 5, 6, 7]
        );
    }

    #[test]
    fn destructuring_and_other_patterns_pass() {
        let text = "fn main() {\n    let paths = pairs.iter().map(|&(path, text)| (path, text));\n    let &[first, ..] = rows.as_slice() else { return };\n    let odd = numbers.iter().filter(|number| *number % 2 != 0);\n    let refs = names.iter().map(|&ref name| name);\n\n    if let Some(&height) = heights.first() {\n        drop(height);\n    }\n\n    let width = match widths.first() {\n        Some(&width) => width,\n        None => 0,\n    };\n}\n";

        assert_eq!(lines_of(Rule::NoReferencePatterns, text), NONE);
    }
}
