//! `no-blocking-in-async`: an `async` body never blocks its thread.
//!
//! A blocking call inside `async` stalls every task on that runtime thread until it returns.
//! Use the runtime's own version (`tokio::fs`, `tokio::time::sleep`) or move the work into
//! `spawn_blocking`. A plain closure inside an async body is a separate context (that's what
//! `spawn_blocking` takes), so it's left alone.

use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{ExprAsync, ExprClosure, File, ImplItemFn, Item, ItemFn};

use super::paths::{file_imports, segments, starts_with};
use super::{Finding, Rule};

const BLOCKING: [(&str, &str); 6] = [
    ("reqwest::blocking", "`reqwest` without `blocking`"),
    ("std::fs", "`tokio::fs`"),
    ("std::net", "`tokio::net`"),
    ("std::process::Command", "`tokio::process::Command`"),
    ("std::thread::sleep", "`tokio::time::sleep`"),
    ("ureq", "an async HTTP client"),
];

struct AsyncBlocking {
    imports: HashMap<String, Vec<String>>,
    async_depth: usize,
    findings: Vec<Finding>,
}

impl AsyncBlocking {
    fn within(&mut self, depth: usize, walk: impl FnOnce(&mut Self)) {
        let outer = std::mem::replace(&mut self.async_depth, depth);
        walk(self);
        self.async_depth = outer;
    }
}

impl<'ast> Visit<'ast> for AsyncBlocking {
    fn visit_expr_async(&mut self, block: &'ast ExprAsync) {
        let depth = self.async_depth + 1;
        self.within(depth, |check| visit::visit_expr_async(check, block));
    }

    fn visit_expr_closure(&mut self, closure: &'ast ExprClosure) {
        let depth = if closure.asyncness.is_some() {
            self.async_depth + 1
        } else {
            0
        };

        self.within(depth, |check| visit::visit_expr_closure(check, closure));
    }

    fn visit_impl_item_fn(&mut self, method: &'ast ImplItemFn) {
        let depth = usize::from(method.sig.asyncness.is_some());
        self.within(depth, |check| visit::visit_impl_item_fn(check, method));
    }

    fn visit_item(&mut self, item: &'ast Item) {
        self.within(0, |check| visit::visit_item(check, item));
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let depth = usize::from(function.sig.asyncness.is_some());
        self.within(depth, |check| visit::visit_item_fn(check, function));
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if self.async_depth > 0 {
            let written = segments(path);

            let full: Vec<String> = match written.first().and_then(|first| self.imports.get(first))
            {
                Some(imported) => imported
                    .iter()
                    .chain(written.iter().skip(1))
                    .cloned()
                    .collect(),
                None => written,
            };

            if let Some((prefix, instead)) = BLOCKING
                .iter()
                .find(|(prefix, _)| starts_with(&full, prefix))
            {
                self.findings.push(Finding {
                    line: path.span().start().line,
                    rule: Rule::NoBlockingInAsync,
                    message: format!("`{prefix}` blocks the runtime thread inside `async`; use {instead}, or `spawn_blocking`"),
                    edit: None,
                });
            }
        }

        visit::visit_path(self, path);
    }
}

pub(super) fn check(file: &File) -> Vec<Finding> {
    let mut check = AsyncBlocking {
        imports: file_imports(file),
        async_depth: 0,
        findings: Vec::new(),
    };
    check.visit_file(file);
    check.findings.dedup_by_key(|finding| finding.line);

    check.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn blocking_inside_async_is_reported() {
        let text = "use std::fs;\n\nasync fn load() {\n    let text = fs::read_to_string(PATH);\n    std::thread::sleep(WAIT);\n}\n";

        assert_eq!(lines_of(Rule::NoBlockingInAsync, text), [4, 5]);
    }

    #[test]
    fn spawn_blocking_closures_are_left_alone() {
        let text = "async fn load() {\n    tokio::task::spawn_blocking(|| std::fs::read(PATH)).await;\n}\n";

        assert_eq!(lines_of(Rule::NoBlockingInAsync, text), NONE);
    }

    #[test]
    fn blocking_outside_async_is_left_alone() {
        let text = "fn load() {\n    std::fs::read(PATH);\n}\n";

        assert_eq!(lines_of(Rule::NoBlockingInAsync, text), NONE);
    }
}
