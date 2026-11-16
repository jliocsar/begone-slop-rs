//! The module tree: which files each crate's targets reach, and as what kind of code.
//!
//! A file pulled in with `#[cfg(test)] mod tests;` is test code even though nothing in its own
//! text says so, and a module of `main.rs` is part of a binary. Walking every target's `mod`
//! declarations from its root file once, before the rules run, is how a rule learns either.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use syn::{Item, ItemMod};

use crate::module_paths::{
    self, FileKind, candidates, children_directory, path_attribute, path_target,
};
use crate::rules::Role;

/// Target roots by where Cargo looks for them, with the kind of code each one is.
const TARGET_DIRECTORIES: [(&str, Role); 4] = [
    ("benches", Role::Bench),
    ("examples", Role::Example),
    ("src/bin", Role::Binary),
    ("tests", Role::Test),
];

#[derive(Default)]
struct Walk {
    roles: HashMap<PathBuf, Role>,
}

/// Where a `mod` declaration sits: its file, the directory that file's modules live in,
/// the inline modules around it, and what kind of code it is.
#[derive(Clone)]
struct Position {
    file: PathBuf,
    children: PathBuf,
    inline: Vec<String>,
    role: Role,
}

impl Walk {
    fn visit(&mut self, file: &Path, children: &Path, role: Role) {
        let known = self.roles.get(file).copied();

        if known.is_some_and(|known| known.is_at_least_as_strict_as(role)) {
            return;
        }

        self.roles.insert(
            file.to_owned(),
            known.map_or(role, |known| known.stricter(role)),
        );

        let Some(parsed) = fs::read_to_string(file)
            .ok()
            .and_then(|text| syn::parse_file(&text).ok())
        else {
            return;
        };

        let position = Position {
            file: file.to_owned(),
            children: children.to_owned(),
            inline: Vec::new(),
            role,
        };

        self.items(&position, &parsed.items);
    }

    fn items(&mut self, position: &Position, items: &[Item]) {
        for module in items
            .iter()
            .filter_map(|item| match item {
                Item::Mod(module) => Some(module),
                _ => None,
            })
        {
            self.module(position, module);
        }
    }

    fn module(&mut self, position: &Position, module: &ItemMod) {
        let role = if module_paths::is_test_only(&module.attrs) {
            Role::Test
        } else {
            position.role
        };

        let name = module.ident.to_string();

        if let Some((_, items)) = &module.content {
            let mut nested = position.clone();

            nested.inline.push(name);
            nested.role = role;
            self.items(&nested, items);

            return;
        }

        if let Some(value) = path_attribute(&module.attrs) {
            let target = path_target(&position.file, &position.children, &position.inline, &value);
            let target_children = target
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_owned();

            if target.is_file() {
                self.visit(&target, &target_children, role);
            }

            return;
        }

        if let Some(child) = candidates(&position.children, &position.inline, &name)
            .into_iter()
            .find(|candidate| candidate.is_file())
        {
            let child_children = children_directory(&child, FileKind::Module);

            self.visit(&child, &child_children, role);
        }
    }
}

/// The directory of the nearest `Cargo.toml` above `file`.
#[must_use]
pub fn crate_root(file: &Path) -> Option<&Path> {
    file.ancestors()
        .skip(1)
        .find(|directory| directory.join("Cargo.toml").is_file())
}

/// The role of every file the targets of the crates around `files` reach. A file reached as
/// two kinds of code gets the stricter one.
#[must_use]
pub fn roles(files: &[PathBuf]) -> HashMap<PathBuf, Role> {
    let mut crates: Vec<&Path> = files
        .iter()
        .filter_map(|file| crate_root(file))
        .collect();

    crates.sort();
    crates.dedup();

    let mut walk = Walk::default();

    for root in crates {
        for (file, role) in targets(root) {
            let children = children_directory(&file, FileKind::TargetRoot);

            walk.visit(&file, &children, role);
        }
    }

    walk.roles
}

/// Every target root of the crate at `root`, by Cargo's conventions.
fn targets(root: &Path) -> Vec<(PathBuf, Role)> {
    let mut found: Vec<(PathBuf, Role)> = [
        ("src/lib.rs", Role::Library),
        ("src/main.rs", Role::Binary),
        ("build.rs", Role::BuildScript),
    ]
    .into_iter()
    .map(|(name, role)| (root.join(name), role))
    .collect();

    for (directory, role) in TARGET_DIRECTORIES {
        let entries = fs::read_dir(root.join(directory))
            .into_iter()
            .flatten()
            .flatten();

        found.extend(entries.filter_map(|entry| {
            let path = entry.path();

            let file = if path.is_dir() {
                path.join("main.rs")
            } else {
                path
            };

            file.extension()
                .is_some_and(|extension| extension == "rs")
                .then_some((file, role))
        }));
    }

    found.retain(|(file, _)| file.is_file());

    found
}
