//! `io-boundary`: effects happen only in the files `bs.toml` allows. Everything else is pure.
//!
//! Functional core, imperative shell, enforced: the file system, network, processes,
//! environment, stdio, clock, randomness and signals are reached only from the shell. An
//! effect is spotted where it's imported, or where a full path names it in code. `Path`'s
//! own file-system methods (`.exists()`, `.canonicalize()`, `.read_dir()`, ...) are effects
//! too, wherever they're called: the receiver's type isn't known here, so any call by those
//! names counts, except `.is_file()`/`.is_dir()`/`.is_symlink()` on what `metadata()` or
//! `file_type()` returned, which only reads a value. Test code is exempt: a test may build
//! its own fixtures on disk.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, ExprMethodCall, ItemUse, Macro};

use super::paths::{UseLeaf, is_pure, macro_arguments, segments, starts_with, use_leaves};
use super::{FileContext, Finding, Rule};
use crate::config::IoBoundary;

const EFFECTS: [(&str, &str); 28] = [
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
    ("std::os::unix::fs", "the file system"),
    ("std::os::unix::net", "the network"),
    ("std::process", "processes"),
    ("std::time::Instant::now", "the clock"),
    ("std::time::SystemTime::now", "the clock"),
    ("tokio::fs", "the file system"),
    ("tokio::io::stderr", "stdio"),
    ("tokio::io::stdin", "stdio"),
    ("tokio::io::stdout", "stdio"),
    ("tokio::net", "the network"),
    ("tokio::process", "processes"),
    ("tokio::signal", "signals"),
    ("tokio::time::Instant::now", "the clock"),
    ("tokio::time::sleep", "the clock"),
    ("std::thread::sleep", "the clock"),
    ("ureq", "the network"),
];

const PRINT_MACROS: [&str; 5] = ["dbg", "eprint", "eprintln", "print", "println"];

/// `Path` and `PathBuf` methods that ask the file system.
const FILE_SYSTEM_METHODS: [&str; 10] = [
    "canonicalize",
    "exists",
    "is_dir",
    "is_file",
    "is_symlink",
    "metadata",
    "read_dir",
    "read_link",
    "symlink_metadata",
    "try_exists",
];

/// Methods also on `fs::FileType` and `fs::Metadata`, where they read a value already fetched.
const KIND_METHODS: [&str; 3] = ["is_dir", "is_file", "is_symlink"];

/// Calls that fetch a `FileType` or `Metadata`.
const KIND_SOURCES: [&str; 3] = ["file_type", "metadata", "symlink_metadata"];

/// Words in a binding's name that say it holds a `FileType` or `Metadata`.
const KIND_NAMES: [&str; 3] = ["kind", "meta", "type"];

struct IoCheck<'run> {
    file: &'run FileContext<'run>,
    boundary: &'run IoBoundary,
    findings: Vec<Finding>,
}

impl IoCheck<'_> {
    fn effect_of(&self, full: &[String]) -> Option<String> {
        if is_pure(full) {
            return None;
        }

        let built_in = EFFECTS
            .iter()
            .find(|(prefix, _)| starts_with(full, prefix))
            .map(|(_, effect)| (*effect).to_owned())
            .or_else(|| is_path_method(full).then(|| "the file system".to_owned()));

        built_in.or_else(|| {
            self.boundary
                .effects
                .iter()
                .find(|prefix| starts_with(full, prefix))
                .map(|prefix| format!("`{prefix}`"))
        })
    }

    fn report(&mut self, line: usize, what: &str, effect: &str) {
        if self.file.is_test(line) {
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
    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        let method = call.method.to_string();
        let reads_a_kind = KIND_METHODS.contains(&method.as_str()) && fetches_kind(&call.receiver);

        if FILE_SYSTEM_METHODS.contains(&method.as_str()) && !reads_a_kind {
            self.report(
                call.method.span().start().line,
                &format!(".{method}()"),
                "the file system",
            );
        }

        visit::visit_expr_method_call(self, call);
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

        for argument in macro_arguments(invocation) {
            self.visit_expr(&argument);
        }

        visit::visit_macro(self, invocation);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let written = segments(path);
        let first = written.first().cloned().unwrap_or_default();

        // An import that is an effect was reported at the `use`; only a path that reaches
        // further through a harmless import (`io::stdout()` after `use std::io`) is new here.
        let full = match self.file.imports.get(&first) {
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

/// `std::path::Path::exists` and the like, written as a path (`Path::exists(&file)`).
fn is_path_method(full: &[String]) -> bool {
    matches!(full, [std, path, owner, method]
        if std == "std"
            && path == "path"
            && (owner == "Path" || owner == "PathBuf")
            && FILE_SYSTEM_METHODS.contains(&method.as_str()))
}

/// Whether `receiver` reads as a `FileType` or `Metadata`: one just fetched
/// (`entry.file_type()?`, `path.metadata()`), or a binding named for one (`kind`,
/// `file_type`, `metadata`).
fn fetches_kind(receiver: &Expr) -> bool {
    match receiver {
        Expr::Try(inner) => fetches_kind(&inner.expr),
        Expr::Paren(inner) => fetches_kind(&inner.expr),
        Expr::Reference(inner) => fetches_kind(&inner.expr),
        Expr::MethodCall(call) => KIND_SOURCES.iter().any(|name| call.method == name),
        Expr::Path(binding) => binding.path.get_ident().is_some_and(|name| {
            let name = name.to_string();

            KIND_NAMES.iter().any(|kind| name.contains(kind))
        }),
        _ => false,
    }
}

fn is_allowed(file: &FileContext, boundary: &IoBoundary) -> bool {
    file.relative.is_some_and(|relative| {
        boundary
            .allowed
            .iter()
            .any(|allowed| relative.starts_with(allowed))
    })
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let Some(boundary) = &file.config.io else {
        return Vec::new();
    };

    if is_allowed(file, boundary) {
        return Vec::new();
    }

    let mut io_check = IoCheck {
        file,
        boundary,
        findings: Vec::new(),
    };

    io_check.visit_file(file.syntax);
    io_check.findings.dedup_by_key(|finding| finding.line);

    io_check.findings
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use super::check;
    use crate::config::{Config, IoBoundary};
    use crate::rules::testing::NONE;
    use crate::rules::{Context, FileContext, Role};
    use crate::source::Source;

    fn io_lines(text: &str) -> Vec<usize> {
        io_lines_in("src/core.rs", text)
    }

    fn io_lines_in(path: &str, text: &str) -> Vec<usize> {
        let config = Config {
            io: Some(IoBoundary {
                allowed: vec![PathBuf::from("src/shell.rs")],
                effects: vec!["sqlx".to_owned()],
            }),
            ..Config::default()
        };
        let source = Source::new(path.into(), text);
        let file = syn::parse_file(text).unwrap();
        let listed = HashSet::new();
        let context = Context {
            config: &config,
            role: Role::Library,
            absolute: PathBuf::from("/project").join(path),
            relative: Some(PathBuf::from(path)),
            listed: &listed,
        };

        check(&FileContext::new(&source, &file, &context))
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
    fn pure_values_under_effect_modules_pass() {
        let text = "use std::net::SocketAddr;\nuse std::process::ExitCode;\nuse std::env::consts::OS;\n\nfn main() -> ExitCode {\n    let local: std::net::IpAddr = LOCAL;\n}\n";

        assert_eq!(io_lines(text), NONE);
    }

    #[test]
    fn effects_inside_macro_arguments_are_reported() {
        let text = "fn sneaky() -> String {\n    format!(\"{}\", std::fs::read_to_string(PATH).unwrap_or_default())\n}\n";

        assert_eq!(io_lines(text), [2]);
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

    #[test]
    fn the_shell_may_touch_the_world() {
        let text = "use std::fs;\n";

        assert_eq!(io_lines_in("src/shell.rs", text), NONE);
        assert_eq!(io_lines_in("src/core.rs", text), [1]);
    }

    #[test]
    fn path_methods_that_ask_the_disk_are_reported() {
        let text = "use std::path::Path;\n\nfn find(path: &Path) -> bool {\n    let real = path.canonicalize();\n    path.exists()\n        && Path::is_dir(path)\n        && path.metadata().is_ok_and(|meta| meta.is_file())\n}\n";

        assert_eq!(io_lines(text), [4, 5, 6, 7]);
    }

    #[test]
    fn reading_a_fetched_kind_isnt_another_effect() {
        let text = "fn kind(entry: &DirEntry) -> bool {\n    entry.file_type().is_ok_and(|kind| kind.is_dir())\n}\n\nfn shell(entry: &DirEntry) -> io::Result<bool> {\n    Ok(entry.file_type()?.is_dir())\n}\n";

        assert_eq!(io_lines(text), NONE);
    }
}
