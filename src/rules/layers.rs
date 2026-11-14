//! `layers`: a crate's modules depend downward only, in the order `bs.toml` declares.
//!
//! Each `[[layers]]` entry names a crate directory and its layers as module paths, top first.
//! A module may import from its own layer and the ones below, never from one above. Every
//! `use` and every path written from `crate`, `self` or `super` is checked. Test code is exempt.
//! Between crates, Cargo's own dependency graph already does this job.

use std::path::{Component, Path};

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{File, ItemFn, ItemMod, ItemUse};

use super::paths::{segments, use_leaves};
use super::test_scope::{is_test_fn, is_test_module};
use super::{Context, Finding, Rule};
use crate::config::Layers;
use crate::source::Source;

const RELATIVE_ROOTS: [&str; 3] = ["crate", "self", "super"];

struct LayerCheck<'config> {
    layers: &'config Layers,
    module: Vec<String>,
    findings: Vec<Finding>,
    test_depth: usize,
}

impl LayerCheck<'_> {
    /// The layer a module path sits in: the longest entry it starts with. Lower is higher up.
    fn layer_of(&self, path: &[String]) -> Option<usize> {
        self.layers
            .order
            .iter()
            .enumerate()
            .filter(|(_, layer)| {
                let wanted: Vec<&str> = layer.split("::").collect();

                path.len() >= wanted.len()
                    && path.iter().zip(&wanted).all(|(have, want)| have == want)
            })
            .max_by_key(|(_, layer)| layer.len())
            .map(|(index, _)| index)
    }

    /// `crate::a`, `self::a` and `super::a` as absolute module paths; anything else is
    /// another crate's and none of this rule's business.
    fn resolve(&self, written: &[String]) -> Option<Vec<String>> {
        let first = written.first()?;

        if !RELATIVE_ROOTS.contains(&first.as_str()) {
            return None;
        }

        if first == "crate" {
            return Some(written.to_vec());
        }

        let mut resolved = self.module.clone();
        let mut rest = written.iter().peekable();

        while rest.next_if(|segment| *segment == "super").is_some() {
            resolved.pop();
        }

        rest.next_if(|segment| *segment == "self");
        resolved.extend(rest.cloned());

        Some(resolved)
    }

    fn check(&mut self, line: usize, written: &[String]) {
        if self.test_depth > 0 {
            return;
        }

        let Some(target) = self.resolve(written) else {
            return;
        };

        let here = self.layer_of(&self.module);
        let there = self.layer_of(&target);

        if let (Some(here), Some(there)) = (here, there)
            && there < here
        {
            let order = self.layers.order.join(" → ");

            self.findings.push(Finding {
                line,
                rule: Rule::Layers,
                message: format!(
                    "`{}` imports `{}` from a layer above its own; dependencies point down ({order})",
                    self.module.join("::"),
                    target.join("::")
                ),
                edit: None,
            });
        }
    }
}

impl<'ast> Visit<'ast> for LayerCheck<'_> {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let is_test = is_test_fn(&function.attrs);
        self.test_depth += usize::from(is_test);
        visit::visit_item_fn(self, function);
        self.test_depth -= usize::from(is_test);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        let is_test = is_test_module(&module.attrs);
        self.test_depth += usize::from(is_test);
        self.module.push(module.ident.to_string());
        visit::visit_item_mod(self, module);
        self.module.pop();
        self.test_depth -= usize::from(is_test);
    }

    fn visit_item_use(&mut self, import: &'ast ItemUse) {
        for leaf in use_leaves(&import.tree) {
            self.check(leaf.line, &leaf.full);
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.check(path.span().start().line, &segments(path));
        visit::visit_path(self, path);
    }
}

/// `src/app/keys.rs` is `crate::app::keys`; `lib.rs`, `main.rs` and `mod.rs` name their
/// directory. Files outside `src/` (tests, examples, benches) aren't modules of the crate.
fn module_path(relative: &Path) -> Option<Vec<String>> {
    let mut parts: Vec<String> = relative
        .strip_prefix("src")
        .ok()?
        .with_extension("")
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str().map(str::to_owned),
            _ => None,
        })
        .collect();

    if parts
        .last()
        .is_some_and(|last| ["lib", "main", "mod"].contains(&last.as_str()))
    {
        parts.pop();
    }

    Some(std::iter::once("crate".to_owned()).chain(parts).collect())
}

pub(super) fn check(source: &Source, file: &File, context: &Context) -> Vec<Finding> {
    let config = context.config;

    if context.in_tests() {
        return Vec::new();
    }

    let Some(relative) = config.relative(&source.path) else {
        return Vec::new();
    };

    config
        .layers
        .iter()
        .filter_map(|layers| {
            let inside = relative.strip_prefix(&layers.crate_root).ok()?;
            let module = module_path(inside)?;
            let mut layer_check = LayerCheck {
                layers,
                module,
                findings: Vec::new(),
                test_depth: 0,
            };
            layer_check.visit_file(file);

            Some(layer_check.findings)
        })
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{LayerCheck, module_path};
    use crate::config::Layers;

    fn violations(module: &str, text: &str) -> Vec<usize> {
        let layers = Layers {
            crate_root: ".".into(),
            order: vec!["crate::conn".to_owned(), "crate::proto".to_owned()],
        };
        let mut layer_check = LayerCheck {
            layers: &layers,
            module: module.split("::").map(str::to_owned).collect(),
            findings: Vec::new(),
            test_depth: 0,
        };
        let file = syn::parse_file(text).unwrap();
        syn::visit::Visit::visit_file(&mut layer_check, &file);

        layer_check
            .findings
            .iter()
            .map(|finding| finding.line)
            .collect()
    }

    #[test]
    fn a_lower_layer_may_not_import_a_higher_one() {
        assert_eq!(
            violations("crate::proto::frame", "use crate::conn::Socket;\n"),
            [1]
        );
    }

    #[test]
    fn a_higher_layer_may_import_a_lower_one() {
        assert_eq!(
            violations("crate::conn", "use crate::proto::{Frame, Header};\n"),
            crate::rules::testing::NONE
        );
    }

    #[test]
    fn super_paths_resolve_from_the_module() {
        let text = "fn send() {\n    super::super::conn::Socket::open();\n}\n";

        assert_eq!(violations("crate::proto::frame", text), [2]);
    }

    #[test]
    fn module_paths_follow_the_file_layout() {
        assert_eq!(
            module_path(Path::new("src/app/keys.rs")),
            Some(vec!["crate".into(), "app".into(), "keys".into()])
        );
        assert_eq!(
            module_path(Path::new("src/lib.rs")),
            Some(vec!["crate".into()])
        );
        assert_eq!(module_path(Path::new("tests/dogfood.rs")), None);
    }
}
