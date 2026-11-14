//! Paths as segment lists: what a `use` brings in, and what a path in code names.

use std::collections::HashMap;

use syn::punctuated::Punctuated;
use syn::{Expr, File, Item, Macro, Path, Token, UseTree};

/// Values that sit under an effectful module but touch nothing: an address is data, and so is
/// an exit code. Naming one isn't an effect.
const PURE: [&str; 16] = [
    "std::env::consts",
    "std::fs::FileType",
    "std::fs::Metadata",
    "std::fs::Permissions",
    "std::net::AddrParseError",
    "std::net::IpAddr",
    "std::net::Ipv4Addr",
    "std::net::Ipv6Addr",
    "std::net::Shutdown",
    "std::net::SocketAddr",
    "std::net::SocketAddrV4",
    "std::net::SocketAddrV6",
    "std::process::ExitCode",
    "std::process::ExitStatus",
    "std::process::Output",
    "std::process::Termination",
];

/// One leaf of a `use` tree: the name it binds here (none for a glob) and the full path.
pub(super) struct UseLeaf {
    pub(super) local: Option<String>,
    pub(super) full: Vec<String>,
    pub(super) line: usize,
}

/// Whether `full` names one of the [`PURE`] values.
pub(super) fn is_pure(full: &[String]) -> bool {
    PURE.iter().any(|prefix| starts_with(full, prefix))
}

/// A macro's arguments, when they read as comma-separated expressions (`format!`, `println!`,
/// `assert_eq!`, `vec!`, `write!`). Anything else comes back empty.
pub(super) fn macro_arguments(invocation: &Macro) -> Vec<Expr> {
    invocation
        .parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
        .map(|arguments| arguments.into_iter().collect())
        .unwrap_or_default()
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
