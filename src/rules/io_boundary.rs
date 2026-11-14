//! `io-boundary`: effects happen only in the files `bs.toml` allows. Everything else is pure.
//!
//! Functional core, imperative shell, enforced: the file system, network, processes,
//! environment, stdio, clock, randomness and signals are reached only from the shell. An
//! effect is spotted where it's imported, or where a full path names it in code. Test code
//! is exempt: a test may build its own fixtures on disk.

use std::collections::HashMap;
use std::path::Path;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{File, ItemFn, ItemMod, ItemUse, Macro};

use super::paths::{UseLeaf, segments, starts_with, use_leaves};
use super::test_scope::{is_test_fn, is_test_module, is_test_path};
use super::{Finding, Rule};
use crate::config::{Config, IoBoundary};
use crate::source::Source;

const EFFECTS: [(&str, &str); 23] = [
    ("chrono::Local::now", "the clock"),
    ("chrono::Utc::now", "the clock"),
    ("fastrand", "randomness"),
    ("getrandom", "randomness"),
    ("rand", "randomness"),
    ("reqwest", "the network"),
    ("std::env", "the environment"),
    ("std::fs", "the file system"),
    ("std::io::stderr", "stdio"),
    ("std::io::stdin", "stdio"),
    ("std::io::stdout", "stdio"),
    ("std::net", "the network"),
    ("std::process", "processes"),
    ("std::time::Instant::now", "the clock"),
    ("std::time::SystemTime::now", "the clock"),
    ("tokio::fs", "the file system"),
    ("tokio::net", "the network"),
    ("tokio::process", "processes"),
    ("tokio::signal", "signals"),
    ("tokio::time::Instant::now", "the clock"),
    ("tokio::time::sleep", "the clock"),
    ("std::thread::sleep", "the clock"),
    ("ureq", "the network"),
];

const PRINT_MACROS: [&str; 4] = ["eprint", "eprintln", "print", "println"];

struct IoCheck<'config> {
    boundary: &'config IoBoundary,
    imports: HashMap<String, Vec<String>>,
    findings: Vec<Finding>,
    test_file: bool,
    test_depth: usize,
}

impl IoCheck<'_> {
    fn in_tests(&self) -> bool {
        self.test_file || self.test_depth > 0
    }

    fn effect_of(&self, full: &[String]) -> Option<String> {
        let built_in = EFFECTS
            .iter()
            .find(|(prefix, _)| starts_with(full, prefix))
            .map(|(_, effect)| (*effect).to_owned());

        built_in.or_else(|| {
            self.boundary
                .effects
                .iter()
                .find(|prefix| starts_with(full, prefix))
                .map(|prefix| format!("`{prefix}`"))
        })
    }

    fn report(&mut self, line: usize, what: &str, effect: &str) {
        if self.in_tests() {
            return;
        }

        self.findings.push(Finding {
            line,
            rule: Rule::IoBoundary,
            message: format!(
                "`{what}` reaches {effect} outside the IO boundary (`[io] allowed` in bs.toml); pass the data in, or move this into the shell"
            ),
            edit: None,
        });
    }

    fn check_leaf(&mut self, leaf: &UseLeaf) {
        if let Some(effect) = self.effect_of(&leaf.full) {
            self.report(leaf.line, &leaf.full.join("::"), &effect);
        }
    }
}

impl<'ast> Visit<'ast> for IoCheck<'_> {
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
            self.check_leaf(&leaf);
        }
    }

    fn visit_macro(&mut self, invocation: &'ast Macro) {
        let name = invocation
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string());

        if let Some(name) = name.filter(|name| PRINT_MACROS.contains(&name.as_str())) {
            self.report(
                invocation.path.span().start().line,
                &format!("{name}!"),
                "stdio",
            );
        }

        visit::visit_macro(self, invocation);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let written = segments(path);
        let first = written.first().cloned().unwrap_or_default();

        // An import that is an effect was reported at the `use`; only a path that reaches
        // further through a harmless import (`io::stdout()` after `use std::io`) is new here.
        let full = match self.imports.get(&first) {
            Some(imported) if self.effect_of(imported).is_some() => None,
            Some(imported) => Some(
                imported
                    .iter()
                    .chain(written.iter().skip(1))
                    .cloned()
                    .collect(),
            ),
            None => Some(written),
        };

        if let Some(full) = full
            && let Some(effect) = self.effect_of(&full)
        {
            self.report(path.span().start().line, &full.join("::"), &effect);
        }

        visit::visit_path(self, path);
    }
}

fn is_allowed(config: &Config, boundary: &IoBoundary, path: &Path) -> bool {
    config.relative(path).is_some_and(|relative| {
        boundary
            .allowed
            .iter()
            .any(|allowed| relative.starts_with(allowed))
    })
}

fn file_imports(file: &File) -> HashMap<String, Vec<String>> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Use(import) => Some(use_leaves(&import.tree)),
            _ => None,
        })
        .flatten()
        .filter_map(|leaf| Some((leaf.local?, leaf.full)))
        .collect()
}

pub(super) fn check(source: &Source, file: &File, config: &Config) -> Vec<Finding> {
    let Some(boundary) = &config.io else {
        return Vec::new();
    };

    if is_allowed(config, boundary, &source.path) {
        return Vec::new();
    }

    let mut io_check = IoCheck {
        boundary,
        imports: file_imports(file),
        findings: Vec::new(),
        test_file: is_test_path(&source.path),
        test_depth: 0,
    };
    io_check.visit_file(file);
    io_check.findings.dedup_by_key(|finding| finding.line);

    io_check.findings
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::check;
    use crate::config::{Config, IoBoundary};
    use crate::rules::testing::NONE;
    use crate::source::Source;

    fn io_lines(text: &str) -> Vec<usize> {
        let config = Config {
            io: Some(IoBoundary {
                allowed: vec![PathBuf::from("src/shell.rs")],
                effects: vec!["sqlx".to_owned()],
            }),
            ..Config::default()
        };
        let source = Source::new("src/core.rs".into(), text);
        let file = syn::parse_file(text).unwrap();

        check(&source, &file, &config)
            .iter()
            .map(|finding| finding.line)
            .collect()
    }

    #[test]
    fn importing_an_effect_is_reported_at_the_use() {
        let text =
            "use std::fs;\nuse std::collections::HashMap;\n\nfn load() {\n    fs::read(PATH);\n}\n";

        assert_eq!(io_lines(text), [1]);
    }

    #[test]
    fn a_full_path_in_code_is_reported_where_it_is() {
        let text = "fn now() {\n    let started = std::time::Instant::now();\n}\n";

        assert_eq!(io_lines(text), [2]);
    }

    #[test]
    fn reaching_through_a_harmless_import_is_reported() {
        let text = "use std::io;\n\nfn greet() {\n    io::stdout();\n    println!(\"hi\");\n}\n";

        assert_eq!(io_lines(text), [4, 5]);
    }

    #[test]
    fn configured_effects_count_too() {
        let text = "use sqlx::query;\n";

        assert_eq!(io_lines(text), [1]);
    }

    #[test]
    fn tests_may_touch_the_disk() {
        let text = "#[cfg(test)]\nmod tests {\n    use std::fs;\n}\n";

        assert_eq!(io_lines(text), NONE);
    }
}
