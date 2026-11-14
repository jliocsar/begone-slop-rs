//! `use-groups`: imports in three groups (std, other crates, this crate), in that order,
//! with a blank line between groups and none inside one.
//!
//! What rustfmt's unstable `group_imports = "StdExternalCrate"` does, on stable. rustfmt then
//! sorts inside each group, since it sorts any run of imports without a blank line.

use syn::visit::{self, Visit};
use syn::{File, Item, ItemMod, ItemUse, UseTree};

use super::gap::{Gap, forbid_blank, require_blank};
use super::{Finding, Rule};
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
}

struct UseGroups<'source> {
    source: &'source Source,
    findings: Vec<Finding>,
}

impl Import {
    fn of(import: &ItemUse) -> Self {
        let span = syn::spanned::Spanned::span(import);

        let root = match &import.tree {
            UseTree::Path(path) if import.leading_colon.is_none() => Some(&path.ident),
            UseTree::Name(name) if import.leading_colon.is_none() => Some(&name.ident),
            UseTree::Rename(rename) if import.leading_colon.is_none() => Some(&rename.ident),
            _ => None,
        };

        let group = match root.map(ToString::to_string) {
            Some(name) if STD_CRATES.contains(&name.as_str()) => Group::Std,
            Some(name) if LOCAL_ROOTS.contains(&name.as_str()) => Group::Local,
            _ => Group::External,
        };

        Self {
            start: span.start().line,
            end: span.end().line,
            group,
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
        let mut run: Vec<Import> = Vec::new();

        for item in items {
            if let Item::Use(import) = item {
                run.push(Import::of(import));
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

            let finding = if previous.group == next.group {
                forbid_blank(self.source, gap, Rule::UseGroups, "inside an import group")
            } else {
                require_blank(self.source, gap, Rule::UseGroups, "between import groups")
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

pub(super) fn check(source: &Source, file: &File) -> Vec<Finding> {
    let mut groups = UseGroups {
        source,
        findings: Vec::new(),
    };
    groups.visit_file(file);

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
    fn a_commented_run_is_reported_but_not_rewritten() {
        let text = "use crate::source::Source;\n// Parsing.\nuse syn::File;\n";

        assert_eq!(fixed(text), text);
        assert_eq!(lines_of(Rule::UseGroups, text), [1]);
    }
}
