//! `no-blocking-in-async`: an `async` body never blocks its thread.
//!
//! A blocking call inside `async` stalls every task on that runtime thread until it returns.
//! Use the runtime's own version (`tokio::fs`, `tokio::time::sleep`) or move the work into
//! `spawn_blocking`. A closure inside an async body runs on the same thread (an iterator's
//! `map`, say), unless it's handed to something that runs it elsewhere: `spawn_blocking`,
//! `block_in_place`, `thread::spawn`, a scope's `spawn`. Macro arguments are read too.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Expr, ExprAsync, ExprCall, ExprClosure, ExprMethodCall, ImplItemFn, Item, ItemFn, Macro,
    TraitItemFn,
};

use super::paths::{Imports, is_pure, macro_arguments, segments, starts_with};
use super::{FileContext, Finding, Rule};

const BLOCKING: [(&str, &str); 6] = [
    ("reqwest::blocking", "`reqwest` without `blocking`"),
    ("std::fs", "`tokio::fs`"),
    ("std::net", "`tokio::net`"),
    ("std::process::Command", "`tokio::process::Command`"),
    ("std::thread::sleep", "`tokio::time::sleep`"),
    ("ureq", "an async HTTP client"),
];

/// Calls that run the closure they're given off the async thread.
const OFFLOADERS: [&str; 3] = ["block_in_place", "spawn", "spawn_blocking"];

struct AsyncBlocking<'run> {
    imports: &'run Imports,
    async_depth: usize,
    findings: Vec<Finding>,
}

impl AsyncBlocking<'_> {
    fn within(&mut self, depth: usize, walk: impl FnOnce(&mut Self)) {
        let outer = std::mem::replace(&mut self.async_depth, depth);

        walk(self);
        self.async_depth = outer;
    }

    /// Visits a call's arguments; a closure handed to an offloader runs off this thread.
    fn arguments<'ast>(&mut self, callee: &str, arguments: impl IntoIterator<Item = &'ast Expr>) {
        let offloads = OFFLOADERS.contains(&callee);

        for argument in arguments {
            match argument {
                Expr::Closure(closure) if offloads => {
                    self.within(0, |check| visit::visit_expr_closure(check, closure));
                }
                _ => self.visit_expr(argument),
            }
        }
    }
}

impl<'ast> Visit<'ast> for AsyncBlocking<'_> {
    fn visit_expr_async(&mut self, block: &'ast ExprAsync) {
        let depth = self.async_depth + 1;

        self.within(depth, |check| visit::visit_expr_async(check, block));
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        let callee = match &*call.func {
            Expr::Path(path) => segments(&path.path).pop().unwrap_or_default(),
            _ => String::new(),
        };

        self.visit_expr(&call.func);
        self.arguments(&callee, &call.args);
    }

    fn visit_expr_closure(&mut self, closure: &'ast ExprClosure) {
        let depth = self.async_depth + usize::from(closure.asyncness.is_some());

        self.within(depth, |check| visit::visit_expr_closure(check, closure));
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        self.visit_expr(&call.receiver);

        if let Some(turbofish) = &call.turbofish {
            self.visit_angle_bracketed_generic_arguments(turbofish);
        }

        self.arguments(&call.method.to_string(), &call.args);
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

    fn visit_trait_item_fn(&mut self, method: &'ast TraitItemFn) {
        let depth = usize::from(method.sig.asyncness.is_some());

        self.within(depth, |check| visit::visit_trait_item_fn(check, method));
    }

    fn visit_macro(&mut self, invocation: &'ast Macro) {
        for argument in macro_arguments(invocation) {
            self.visit_expr(&argument);
        }

        visit::visit_macro(self, invocation);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if self.async_depth > 0 {
            let full = self.imports.resolve(&segments(path));
            let blocking = BLOCKING
                .iter()
                .find(|(prefix, _)| starts_with(&full, prefix))
                .filter(|_| !is_pure(&full));

            if let Some((prefix, instead)) = blocking {
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

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut check = AsyncBlocking {
        imports: &file.imports,
        async_depth: 0,
        findings: Vec::new(),
    };

    check.visit_file(file.syntax);
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
    fn closures_that_run_in_place_are_read() {
        let text = "async fn read_all(paths: &[PathBuf]) -> Vec<String> {\n    paths.iter().map(|path| std::fs::read_to_string(path)).collect()\n}\n\nasync fn offload() {\n    std::thread::scope(|scope| scope.spawn(|| std::fs::read(PATH)));\n}\n";

        assert_eq!(lines_of(Rule::NoBlockingInAsync, text), [2]);
    }

    #[test]
    fn macro_arguments_and_trait_defaults_are_read() {
        let text = "async fn show(path: &Path) {\n    println!(\"{}\", std::fs::read_to_string(path).unwrap_or_default());\n}\n\ntrait Store {\n    async fn load(&self) -> Vec<u8> {\n        std::fs::read(PATH).unwrap_or_default()\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoBlockingInAsync, text), [2, 7]);
    }

    #[test]
    fn addresses_are_values() {
        let text = "async fn connect(address: std::net::SocketAddr) {\n    let local: std::net::IpAddr = LOCAL;\n}\n";

        assert_eq!(lines_of(Rule::NoBlockingInAsync, text), NONE);
    }

    #[test]
    fn blocking_outside_async_is_left_alone() {
        let text = "fn load() {\n    std::fs::read(PATH);\n}\n";

        assert_eq!(lines_of(Rule::NoBlockingInAsync, text), NONE);
    }
}
