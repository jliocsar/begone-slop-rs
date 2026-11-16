//! `reexport-only-module`: a module file that only re-exports is a barrel.
//!
//! It adds a node to the import graph that owns nothing. Import from where the items live.
//! `lib.rs` and `main.rs` are exempt: a crate root's re-exports are its public API.

use syn::{Item, Visibility};

use super::{FileContext, Finding, Rule};

const CRATE_ROOTS: [&str; 2] = ["lib.rs", "main.rs"];

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let syntax = file.syntax;
    let is_root = file
        .source
        .path
        .file_name()
        .is_some_and(|name| {
            CRATE_ROOTS
                .iter()
                .any(|root| name == *root)
        });
    let only_uses = !syntax.items.is_empty()
        && syntax
            .items
            .iter()
            .all(|item| matches!(item, Item::Use(_)));
    let reexports = syntax.items.iter().any(
        |item| matches!(item, Item::Use(import) if !matches!(import.vis, Visibility::Inherited)),
    );

    (!is_root && only_uses && reexports)
        .then(|| Finding {
            line: 1,
            rule: Rule::ReexportOnly,
            message: "this module only re-exports; import from where the items live".to_owned(),
            edit: None,
        })
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::findings;

    #[test]
    fn a_barrel_module_is_reported() {
        let found = findings("src/shapes.rs", "pub use crate::circle::Circle;\n");

        assert!(
            found
                .iter()
                .any(|finding| finding.rule == Rule::ReexportOnly)
        );
    }

    #[test]
    fn a_crate_root_may_re_export() {
        let found = findings("src/lib.rs", "pub use crate::circle::Circle;\n");

        assert!(
            found
                .iter()
                .all(|finding| finding.rule != Rule::ReexportOnly)
        );
    }
}
