//! Paths as segment lists: what a `use` brings in, and what a path in code names.

use std::collections::HashMap;

use syn::{File, Item, Path, UseTree};

/// One leaf of a `use` tree: the name it binds here (none for a glob) and the full path.
pub(super) struct UseLeaf {
    pub(super) local: Option<String>,
    pub(super) full: Vec<String>,
    pub(super) line: usize,
}

pub(super) fn segments(path: &Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

/// `use a::{b, c::d as e, f::*}` gives `b → a::b`, `e → a::c::d` and a glob `a::f`.
pub(super) fn use_leaves(tree: &UseTree) -> Vec<UseLeaf> {
    let mut leaves = Vec::new();
    collect(tree, &[], &mut leaves);

    leaves
}

/// `full` starts with `prefix` on a segment boundary.
pub(super) fn starts_with(full: &[String], prefix: &str) -> bool {
    let wanted: Vec<&str> = prefix.split("::").collect();

    full.len() >= wanted.len() && full.iter().zip(&wanted).all(|(have, want)| have == want)
}

/// What each name imported at the top of the file stands for: `fs` → `std::fs`.
pub(super) fn file_imports(file: &File) -> HashMap<String, Vec<String>> {
    file.items
        .iter()
        .filter_map(|item| match item {
            Item::Use(import) => Some(use_leaves(&import.tree)),
            _ => None,
        })
        .flatten()
        .filter_map(|leaf| Some((leaf.local?, leaf.full)))
        .collect()
}

fn collect(tree: &UseTree, prefix: &[String], leaves: &mut Vec<UseLeaf>) {
    let joined = |name: String| {
        let mut full = prefix.to_vec();
        full.push(name);

        full
    };

    match tree {
        UseTree::Path(path) => collect(&path.tree, &joined(path.ident.to_string()), leaves),
        UseTree::Name(name) if name.ident == "self" => leaves.push(UseLeaf {
            local: prefix.last().cloned(),
            full: prefix.to_vec(),
            line: name.ident.span().start().line,
        }),
        UseTree::Name(name) => leaves.push(UseLeaf {
            local: Some(name.ident.to_string()),
            full: joined(name.ident.to_string()),
            line: name.ident.span().start().line,
        }),
        UseTree::Rename(rename) => leaves.push(UseLeaf {
            local: Some(rename.rename.to_string()),
            full: if rename.ident == "self" {
                prefix.to_vec()
            } else {
                joined(rename.ident.to_string())
            },
            line: rename.ident.span().start().line,
        }),
        UseTree::Glob(glob) => leaves.push(UseLeaf {
            local: None,
            full: prefix.to_vec(),
            line: glob.star_token.spans[0].start().line,
        }),
        UseTree::Group(group) => {
            for item in &group.items {
                collect(item, prefix, leaves);
            }
        }
    }
}
