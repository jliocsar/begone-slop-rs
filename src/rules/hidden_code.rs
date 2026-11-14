//! `no-hidden-code`: every line of the crate's code sits where the checks read it.
//!
//! `#[path = "core.inc"] mod core;` and `include!("more.in")` compile code no rule ever sees:
//! the run checks the `.rs` files git lists, and those aren't among them. The same goes for a
//! `.rs` file git ignores. `include!` of a path built at compile time can point anywhere, so
//! only Cargo's own generated-code idiom, `include!(concat!(env!("OUT_DIR"), ...))`, passes:
//! the build script that writes it is checked. This guards the gate, so it can't be waived.

use std::path::Path;

use quote::ToTokens;
use syn::visit::{self, Visit};
use syn::{Expr, ExprLit, ItemMod, Lit, Macro};

use super::{FileContext, Finding, Rule};
use crate::module_paths::{FileKind, children_directory, normalize, path_attribute, path_target};

struct HiddenCode<'run> {
    file: &'run FileContext<'run>,
    children: std::path::PathBuf,
    inline: Vec<String>,
    findings: Vec<Finding>,
}

impl HiddenCode<'_> {
    /// Why code at `target` (written as `written`) is hidden, if it is.
    fn hidden(&self, target: &Path, written: &str) -> Option<String> {
        if target.extension().is_none_or(|extension| extension != "rs") {
            Some(format!(
                "`{written}`, which isn't a `.rs` file, so no rule reads the code in it; make it a `.rs` module"
            ))
        } else if !self.file.listed.contains(target) {
            Some(format!(
                "`{written}`, which isn't among the files git lists, so nothing checks it; track it, or move the code into the crate"
            ))
        } else {
            None
        }
    }

    /// `include!("file")` must name a checked `.rs` file; a computed path must be `OUT_DIR`'s.
    fn include(&mut self, invocation: &Macro, line: usize) {
        match invocation.parse_body::<Expr>() {
            Ok(Expr::Lit(ExprLit {
                lit: Lit::Str(text),
                ..
            })) => {
                let directory = self
                    .file
                    .absolute
                    .parent()
                    .unwrap_or_else(|| Path::new(""));
                let target = normalize(&directory.join(text.value()));

                if let Some(message) = self.hidden(&target, &text.value()) {
                    self.push(line, format!("`include!` pulls in {message}"));
                }
            }
            Ok(Expr::Macro(inner)) if is_out_dir(&inner.mac) => {}
            Ok(_) | Err(_) => self.push(
                line,
                "`include!` of a computed path can pull in code nothing checks; only `include!(concat!(env!(\"OUT_DIR\"), ...))` from a build script may".to_owned(),
            ),
        }
    }

    fn push(&mut self, line: usize, message: String) {
        self.findings.push(Finding {
            line,
            rule: Rule::NoHiddenCode,
            message,
            edit: None,
        });
    }
}

impl<'ast> Visit<'ast> for HiddenCode<'_> {
    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if module.content.is_none()
            && let Some(value) = path_attribute(&module.attrs)
        {
            let target = path_target(self.file.absolute, &self.children, &self.inline, &value);

            if let Some(message) = self.hidden(&target, &value) {
                self.push(
                    module.mod_token.span.start().line,
                    format!("`#[path]` points at {message}"),
                );
            }
        }

        self.inline.push(module.ident.to_string());
        visit::visit_item_mod(self, module);
        self.inline.pop();
    }

    fn visit_macro(&mut self, invocation: &'ast Macro) {
        let is_include = invocation
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "include");

        if is_include {
            let line = invocation
                .path
                .segments
                .first()
                .map_or(1, |segment| segment.ident.span().start().line);

            self.include(invocation, line);
        }

        visit::visit_macro(self, invocation);
    }
}

/// `concat!(env!("OUT_DIR"), ...)`: code a build script generated.
fn is_out_dir(invocation: &Macro) -> bool {
    let tokens = invocation
        .tokens
        .to_token_stream()
        .to_string()
        .replace(' ', "");

    invocation.path.is_ident("concat") && tokens.starts_with("env!(\"OUT_DIR\")")
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut hidden = HiddenCode {
        file,
        children: children_directory(file.absolute, FileKind::Module),
        inline: Vec::new(),
        findings: Vec::new(),
    };
    hidden.visit_file(file.syntax);

    hidden.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, findings_listing};

    fn hidden_lines(text: &str, listed: &[&str]) -> Vec<usize> {
        findings_listing("src/lib.rs", text, listed)
            .into_iter()
            .filter(|finding| finding.rule == Rule::NoHiddenCode)
            .map(|finding| finding.line)
            .collect()
    }

    #[test]
    fn code_outside_rs_files_is_reported() {
        let text = "#[path = \"core.inc\"]\nmod core_logic;\n\ninclude!(\"more.in\");\n";

        assert_eq!(hidden_lines(text, &[]), [2, 4]);
    }

    #[test]
    fn rs_files_git_doesnt_list_are_reported() {
        let text =
            "#[path = \"generated/model.rs\"]\nmod model;\n\n#[path = \"wire.rs\"]\nmod wire;\n";

        assert_eq!(hidden_lines(text, &["src/wire.rs"]), [2]);
    }

    #[test]
    fn generated_code_from_out_dir_passes() {
        let text = "include!(concat!(env!(\"OUT_DIR\"), \"/bindings.rs\"));\n\nfn load() {\n    include!(make_path());\n}\n";

        assert_eq!(hidden_lines(text, &[]), [4]);
    }

    #[test]
    fn plain_modules_pass() {
        let text = "mod app;\n\nconst NOTE: &str = include_str!(\"note.txt\");\n";

        assert_eq!(hidden_lines(text, &[]), NONE);
    }
}
