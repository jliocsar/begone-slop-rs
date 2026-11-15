//! `use-groups`: imports in three groups (std, other crates, this crate), in that order,
//! with a blank line between groups and none inside one.
//!
//! What rustfmt's unstable `group_imports = "StdExternalCrate"` does, on stable. rustfmt then
//! sorts inside each group, since it sorts any run of imports without a blank line.

use syn::visit::{self, Visit};
use syn::{File, Item, ItemMod, ItemUse, UseTree, Visibility};

use super::gap::{Gap, forbid_blank, require_blank};
use super::{FileContext, Finding, Rule};
use crate::source::{Edit, Source};

const STD_CRATES: [&str; 5] = ["alloc", "core", "proc_macro", "std", "test"];
const LOCAL_ROOTS: [&str; 3] = ["crate", "self", "super"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Std,
    External,
    Local,
}

#[derive(Clone, Copy, Debug)]
struct Import {
    start: usize,
    end: usize,
    group: Group,
    /// A `pub use`: re-exports may sit apart from the imports in their group.
    exported: bool,
}

struct UseGroups<'source> {
    source: &'source Source,
    findings: Vec<Finding>,
}

impl Import {
    /// `locals` are the modules and enums declared next to the import: `use child::Thing`
    /// names this crate's code, just as `use self::child::Thing` would.
    fn of(import: &ItemUse, locals: &[String]) -> Self {
        let span = syn::spanned::Spanned::span(import);

        let root = match &import.tree {
            UseTree::Path(path) => Some(&path.ident),
            UseTree::Name(name) => Some(&name.ident),
            UseTree::Rename(rename) => Some(&rename.ident),
            UseTree::Glob(_) | UseTree::Group(_) => None,
        };

        let root = root.map(ToString::to_string);
        let absolute = import.leading_colon.is_some();

        let group = match root.as_deref() {
            Some(name) if STD_CRATES.contains(&name) => Group::Std,
            Some(_) if absolute => Group::External,
            Some(name)
                if LOCAL_ROOTS.contains(&name) || locals.iter().any(|local| local == name) =>
            {
                Group::Local
            }
            Some(_) | None => Group::External,
        };

        Self {
            start: span.start().line,
            end: span.end().line,
            group,
            exported: !matches!(import.vis, Visibility::Inherited),
        }
    }

    fn gap_to(self, next: Self) -> Gap {
        Gap {
            after: self.end,
            before: next.start,
        }
    }
}

impl UseGroups<'_> {
    fn items(&mut self, items: &[Item]) {
        let locals: Vec<String> = items
            .iter()
            .filter_map(|item| match item {
                Item::Mod(module) => Some(module.ident.to_string()),
                Item::Enum(declared) => Some(declared.ident.to_string()),
                _ => None,
            })
            .collect();
        let mut run: Vec<Import> = Vec::new();

        for item in items {
            if let Item::Use(import) = item {
                run.push(Import::of(import, &locals));
            } else {
                self.run(&std::mem::take(&mut run));
            }
        }

        self.run(&run);
    }

    fn run(&mut self, imports: &[Import]) {
        let in_order = imports.is_sorted_by_key(|import| import.group);

        if !in_order {
            self.findings.extend(self.regroup(imports));

            return;
        }

        for pair in imports.windows(2) {
            let [previous, next] = pair else {
                continue;
            };

            let gap = previous.gap_to(*next);

            let finding = if previous.group != next.group {
                require_blank(self.source, gap, Rule::UseGroups, "between import groups")
            } else if previous.exported == next.exported {
                forbid_blank(self.source, gap, Rule::UseGroups, "inside an import group")
            } else {
                None
            };

            self.findings.extend(finding);
        }
    }

    /// Rewrites the run in group order when nothing but imports and blank lines sit in it;
    /// a comment could belong to either neighbour, so a run with one is only reported.
    fn regroup(&self, imports: &[Import]) -> Option<Finding> {
        let (first, last) = (imports.first()?, imports.last()?);
        let only_imports = imports.windows(2).all(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(previous, next)| {
                    (previous.end + 1..next.start).all(|number| self.source.is_blank(number))
                })
        });
        let mut sorted = imports.to_vec();

        sorted.sort_by_key(|import| import.group);

        let mut lines: Vec<String> = Vec::new();

        for (index, import) in sorted.iter().enumerate() {
            let starts_group = index > 0
                && sorted
                    .get(index - 1)
                    .is_some_and(|previous| previous.group != import.group);

            if starts_group {
                lines.push(String::new());
            }

            lines.extend(
                (import.start..=import.end)
                    .filter_map(|number| self.source.line(number).map(str::to_owned)),
            );
        }

        Some(Finding {
            line: first.start,
            rule: Rule::UseGroups,
            message: "imports go std, then other crates, then this crate (`crate`, `super`, `self`), with a blank line between groups".to_owned(),
            edit: only_imports.then_some(Edit::Replace {
                first: first.start,
                last: last.end,
                lines,
            }),
        })
    }
}

impl<'ast> Visit<'ast> for UseGroups<'_> {
    fn visit_file(&mut self, file: &'ast File) {
        self.items(&file.items);
        visit::visit_file(self, file);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if let Some((_, items)) = &module.content {
            self.items(items);
        }

        visit::visit_item_mod(self, module);
    }
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut groups = UseGroups {
        source: file.source,
        findings: Vec::new(),
    };

    groups.visit_file(file.syntax);

    groups.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, fixed, lines_of};

    #[test]
    fn grouped_imports_pass() {
        let text = "use std::fs;\nuse std::io;\n\nuse syn::File;\n\nuse crate::source::Source;\n";

        assert_eq!(lines_of(Rule::UseGroups, text), NONE);
    }

    #[test]
    fn groups_need_a_blank_line_between_them() {
        let text = "use std::fs;\nuse syn::File;\n";

        assert_eq!(lines_of(Rule::UseGroups, text), [2]);
    }

    #[test]
    fn a_group_has_no_blank_line_inside() {
        let text = "use std::fs;\n\nuse std::io;\n";

        assert_eq!(lines_of(Rule::UseGroups, text), [3]);
    }

    #[test]
    fn imports_out_of_order_are_regrouped() {
        let text = "use crate::source::Source;\nuse syn::{\n    File,\n    Item,\n};\nuse std::fs;\n\nfn run() {}\n";

        assert_eq!(
            fixed(text),
            "use std::fs;\n\nuse syn::{\n    File,\n    Item,\n};\n\nuse crate::source::Source;\n\nfn run() {}\n"
        );
    }

    #[test]
    fn a_child_module_is_this_crate() {
        let text =
            "mod hooks;\n\nuse rune::Key;\n\npub use hooks::run;\n\nuse crate::pane::CellSize;\n";

        assert_eq!(lines_of(Rule::UseGroups, text), NONE);
    }

    #[test]
    fn re_exports_may_sit_apart_in_their_group() {
        let text =
            "use crate::pane::CellSize;\n\npub use crate::pane::Grids;\nuse crate::term::Size;\n";

        assert_eq!(lines_of(Rule::UseGroups, text), NONE);
    }

    #[test]
    fn an_absolute_path_is_another_crate_unless_its_std() {
        let text = "use ::std::fs;\n\nuse ::serde::Serialize;\nuse syn::File;\n";

        assert_eq!(lines_of(Rule::UseGroups, text), NONE);
    }

    #[test]
    fn a_commented_run_is_reported_but_not_rewritten() {
        let text = "use crate::source::Source;\n// Parsing.\nuse syn::File;\n";

        assert_eq!(fixed(text), text);
        assert_eq!(lines_of(Rule::UseGroups, text), [1]);
    }
}
