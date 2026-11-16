//! Where a `mod x;` finds its file: `x.rs`, `x/mod.rs`, or what its `#[path]` says. Pure path
//! arithmetic; whether the file exists is the caller's question.

use std::path::{Component, Path, PathBuf};

use syn::{Attribute, Expr, Lit, Meta};

/// File names whose `mod x;` declarations resolve next to them, not in a directory named
/// after them.
const MOD_RS_NAMES: [&str; 3] = ["lib.rs", "main.rs", "mod.rs"];

/// Whether a file is a target's root (`tests/cli.rs`, `src/bin/tool.rs`) or a module below one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Module,
    TargetRoot,
}

/// The directory a file's `mod x;` declarations resolve in: its own for `lib.rs`, `main.rs`,
/// `mod.rs` and other target roots, `a/b/` for `a/b.rs`.
#[must_use]
pub fn children_directory(file: &Path, kind: FileKind) -> PathBuf {
    let directory = file
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_owned();
    let mod_rs = kind == FileKind::TargetRoot
        || file.file_name().is_some_and(|name| {
            MOD_RS_NAMES
                .iter()
                .any(|known| name == *known)
        });

    match file.file_stem() {
        Some(stem) if !mod_rs => directory.join(stem),
        _ => directory,
    }
}

/// The two files `mod name;` may live in, inside the inline modules `inline`.
#[must_use]
pub fn candidates(children: &Path, inline: &[String], name: &str) -> [PathBuf; 2] {
    let base = inline
        .iter()
        .fold(children.to_owned(), |path, module| path.join(module));

    [
        base.join(format!("{name}.rs")),
        base.join(name).join("mod.rs"),
    ]
}

/// Where `#[path = "value"]` points. Outside inline modules it's relative to the file's own
/// directory; inside them, to the children directory plus the inline modules.
#[must_use]
pub fn path_target(file: &Path, children: &Path, inline: &[String], value: &str) -> PathBuf {
    let base = if inline.is_empty() {
        file.parent()
            .unwrap_or_else(|| Path::new(""))
            .to_owned()
    } else {
        inline
            .iter()
            .fold(children.to_owned(), |path, module| path.join(module))
    };

    normalize(&base.join(value))
}

/// The string in `#[path = "..."]`, if the attributes have one.
#[must_use]
pub fn path_attribute(attributes: &[Attribute]) -> Option<String> {
    attributes
        .iter()
        .find_map(|attribute| match &attribute.meta {
            Meta::NameValue(pair) if pair.path.is_ident("path") => match &pair.value {
                Expr::Lit(literal) => match &literal.lit {
                    Lit::Str(text) => Some(text.value()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        })
}

/// Whether the attributes compile the item only for tests: `#[cfg(test)]`, or `test` inside
/// `cfg(all(...))`.
#[must_use]
pub fn is_test_only(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| match &attribute.meta {
            Meta::List(list) if list.path.is_ident("cfg") => list
                .parse_args::<Meta>()
                .is_ok_and(|predicate| requires_test(&predicate)),
            _ => false,
        })
}

fn requires_test(predicate: &Meta) -> bool {
    match predicate {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) if list.path.is_ident("all") => list
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
            .is_ok_and(|parts| parts.iter().any(requires_test)),
        _ => false,
    }
}

/// `a/./b/../c` as `a/c`, without asking the file system.
#[must_use]
pub fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normal.pop() {
                    normal.push("..");
                }
            }
            other => normal.push(other),
        }
    }

    normal
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{FileKind, candidates, children_directory, is_test_only, normalize, path_target};

    #[test]
    fn children_of_a_non_mod_rs_file_live_in_its_directory() {
        assert_eq!(
            children_directory(Path::new("src/app.rs"), FileKind::Module),
            Path::new("src/app")
        );
        assert_eq!(
            children_directory(Path::new("src/lib.rs"), FileKind::Module),
            Path::new("src")
        );
        assert_eq!(
            children_directory(Path::new("tests/cli.rs"), FileKind::TargetRoot),
            Path::new("tests")
        );
    }

    #[test]
    fn a_module_has_two_candidate_files() {
        let [flat, nested] = candidates(Path::new("src/app"), &["inner".to_owned()], "keys");

        assert_eq!(flat, Path::new("src/app/inner/keys.rs"));
        assert_eq!(nested, Path::new("src/app/inner/keys/mod.rs"));
    }

    #[test]
    fn a_path_attribute_resolves_from_the_file() {
        let target = path_target(
            Path::new("src/lib.rs"),
            Path::new("src"),
            &[],
            "../gen/core.inc",
        );

        assert_eq!(target, PathBuf::from("gen/core.inc"));
        assert_eq!(normalize(Path::new("/a/./b/../c")), Path::new("/a/c"));
    }

    #[test]
    fn all_with_test_is_test_only() {
        let only = |text: &str| {
            is_test_only(
                &syn::parse_str::<syn::ItemMod>(text)
                    .unwrap()
                    .attrs,
            )
        };

        assert!(only("#[cfg(test)] mod tests;"));
        assert!(only("#[cfg(all(test, feature = \"x\"))] mod tests;"));
        assert!(!only("#[cfg(any(test, feature = \"x\"))] mod tests;"));
        assert!(!only("#[cfg(feature = \"x\")] mod tests;"));
    }
}
