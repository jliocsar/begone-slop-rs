//! `no-forbidden-expects` and `no-crate-level-allow`: waivers a reason can't buy.
//!
//! Some lints guard the gate itself: waive `unwrap_used` or `dead_code` and the slop they catch
//! walks back in with a polite reason attached. Outside tests, those lints can't be waived at
//! all: the ones below, every lint the preset turns on by name, every group (a group waives its
//! members), the `unused` family, and the lints that let an old or unknown name through. Old
//! names count as their current ones. And no lint is waived for a whole file, module or crate
//! (`#![allow]`, `#[allow] mod x;`), behind a `cfg_attr` (at any depth), or from a macro body,
//! where a waiver hides from review.

use std::sync::LazyLock;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{AttrStyle, Attribute, ItemMod, Macro, Meta, MetaList, Token};

use super::paths::segments;
use super::test_scope::TestLines;
use super::{FileContext, Finding, Rule};
use crate::lints::{self, Level};
use crate::preset;

const FORBIDDEN: [&str; 24] = [
    "clippy::all",
    "clippy::allow_attributes",
    "clippy::allow_attributes_without_reason",
    "clippy::dbg_macro",
    "clippy::exit",
    "clippy::expect_used",
    "clippy::panic",
    "clippy::pedantic",
    "clippy::todo",
    "clippy::undocumented_unsafe_blocks",
    "clippy::unimplemented",
    "clippy::unreachable",
    "clippy::unused_result_ok",
    "clippy::unwrap_used",
    "dead_code",
    "renamed_and_removed_lints",
    "unknown_lints",
    "unsafe_code",
    "unused",
    "unused_imports",
    "unused_must_use",
    "unused_mut",
    "unused_variables",
    "warnings",
];

const LEVELS: [&str; 2] = ["allow", "expect"];

/// The lints the preset turns on by name. A preset that doesn't parse is a drift finding, so
/// here it only means a shorter list.
static PRESET: LazyLock<Vec<String>> = LazyLock::new(|| {
    preset::lint_entries()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.level >= Level::Warn)
        .map(|entry| entry.name)
        .collect()
});

/// Where a waiver sits, which decides how much it covers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// On an item: covers that item.
    Item,
    /// On an item behind `cfg_attr`: covers it in some builds only.
    Conditional,
    /// `#![allow]`: covers the file, module or crate it's in.
    Inner,
    /// On a `mod`: covers the module, which for `mod x;` is a whole file.
    Module,
    /// Inside a macro's tokens, where it lands wherever the macro expands.
    Macro,
}

struct LintWaivers<'run> {
    tests: &'run TestLines,
    findings: Vec<Finding>,
}

impl LintWaivers<'_> {
    fn push(&mut self, line: usize, rule: Rule, message: String) {
        self.findings.push(Finding {
            line,
            rule,
            message,
            edit: None,
        });
    }

    fn check_levels(&mut self, line: usize, list: &MetaList) {
        if self.tests.contains(line) {
            return;
        }

        for lint in waived_lints(list) {
            if is_forbidden(&lint) {
                self.push(
                    line,
                    Rule::NoForbiddenExpects,
                    format!("`{lint}` can't be waived outside tests, not even with a reason; fix what it reports"),
                );
            }
        }
    }

    fn check_meta(&mut self, line: usize, placement: Placement, meta: &Meta) {
        let Meta::List(list) = meta else {
            return;
        };

        if is_level(list) {
            self.check_waiver(line, placement, list);
        } else if list.path.is_ident("cfg_attr") {
            let placement = match placement {
                Placement::Item => Placement::Conditional,
                wider @ (Placement::Conditional
                | Placement::Inner
                | Placement::Module
                | Placement::Macro) => wider,
            };

            for waiver in conditional_levels(list) {
                self.check_waiver(line, placement, &waiver);
            }
        }
    }

    fn check_waiver(&mut self, line: usize, placement: Placement, waiver: &MetaList) {
        let wide = match placement {
            Placement::Item => None,
            Placement::Conditional => Some(
                "a waiver behind `cfg_attr` only applies in some builds and hides from review; waive the item directly, or not at all",
            ),
            Placement::Inner => Some(
                "a waiver for a whole file or module hides every case it covers; put `#[expect(lint, reason = \"...\")]` on the item that needs it",
            ),
            Placement::Module => Some(
                "a waiver on a module covers every item in it, and on `mod x;` a whole file; put `#[expect(lint, reason = \"...\")]` on the item that needs it",
            ),
            Placement::Macro => Some(
                "a waiver inside a macro lands on whatever the macro expands to, out of review's sight; waive the item where it's written",
            ),
        };

        if let Some(message) = wide {
            self.push(line, Rule::NoCrateLevelAllow, message.to_owned());
        }

        self.check_levels(line, waiver);
    }

    fn check_attribute(&mut self, attribute: &Attribute, outer: Placement) {
        let placement = match attribute.style {
            AttrStyle::Inner(_) => Placement::Inner,
            AttrStyle::Outer => outer,
        };

        self.check_meta(
            attribute.pound_token.span.start().line,
            placement,
            &attribute.meta,
        );
    }
}

impl<'ast> Visit<'ast> for LintWaivers<'_> {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        self.check_attribute(attribute, Placement::Item);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        for attribute in &module.attrs {
            self.check_attribute(attribute, Placement::Module);
        }

        for item in module
            .content
            .iter()
            .flat_map(|(_, items)| items)
        {
            self.visit_item(item);
        }
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        for (line, meta) in attributes_in(mac.tokens.clone()) {
            self.check_meta(line, Placement::Macro, &meta);
        }

        visit::visit_macro(self, mac);
    }
}

/// Whether waiving `lint` is off limits outside tests.
fn is_forbidden(lint: &str) -> bool {
    let lint = lints::current_name(lint);
    let rustc_unused = lint.starts_with("unused");

    FORBIDDEN.contains(&lint)
        || rustc_unused
        || lints::is_group(lint)
        || PRESET.iter().any(|name| name == lint)
}

fn is_level(list: &MetaList) -> bool {
    LEVELS
        .iter()
        .any(|level| list.path.is_ident(level))
}

fn arguments(list: &MetaList) -> Vec<Meta> {
    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .map(|metas| metas.into_iter().collect())
        .unwrap_or_default()
}

fn waived_lints(list: &MetaList) -> Vec<String> {
    arguments(list)
        .iter()
        .filter_map(|meta| match meta {
            Meta::Path(path) => Some(segments(path).join("::")),
            Meta::List(_) | Meta::NameValue(_) => None,
        })
        .collect()
}

/// The `allow`/`expect` lists inside `cfg_attr(condition, ...)`, through any nesting.
fn conditional_levels(list: &MetaList) -> Vec<MetaList> {
    arguments(list)
        .into_iter()
        .skip(1)
        .flat_map(|meta| match meta {
            Meta::List(inner) if is_level(&inner) => vec![inner],
            Meta::List(inner) if inner.path.is_ident("cfg_attr") => conditional_levels(&inner),
            Meta::Path(_) | Meta::List(_) | Meta::NameValue(_) => Vec::new(),
        })
        .collect()
}

/// `#[...]` and `#![...]` attributes written inside macro tokens, with their lines.
fn attributes_in(tokens: TokenStream) -> Vec<(usize, Meta)> {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut found = Vec::new();

    for (index, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Punct(pound) if pound.as_char() == '#' => {
                let bang = matches!(trees.get(index + 1), Some(TokenTree::Punct(bang)) if bang.as_char() == '!');
                let next = trees.get(index + 1 + usize::from(bang));

                if let Some(TokenTree::Group(group)) = next
                    && group.delimiter() == Delimiter::Bracket
                    && let Ok(meta) = syn::parse2::<Meta>(group.stream())
                {
                    found.push((pound.span().start().line, meta));
                }
            }
            TokenTree::Group(group) => found.extend(attributes_in(group.stream())),
            TokenTree::Punct(_) | TokenTree::Ident(_) | TokenTree::Literal(_) => {}
        }
    }

    found
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut waivers = LintWaivers {
        tests: &file.tests,
        findings: Vec::new(),
    };

    waivers.visit_file(file.syntax);

    waivers.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn guarding_lints_cant_be_waived() {
        let text = "#[expect(clippy::unwrap_used, reason = \"it's fine\")]\nfn load() {}\n\n#[expect(clippy::cast_possible_truncation, reason = \"fits in u8\")]\nfn narrow() {}\n";

        assert_eq!(lines_of(Rule::NoForbiddenExpects, text), [1]);
    }

    #[test]
    fn tests_may_waive_them() {
        let text = "#[cfg(test)]\nmod tests {\n    #[expect(dead_code, reason = \"fixture\")]\n    fn unused() {}\n}\n";

        assert_eq!(lines_of(Rule::NoForbiddenExpects, text), NONE);
    }

    #[test]
    fn groups_old_names_and_preset_lints_cant_be_waived() {
        let text = "#[allow(clippy::restriction)]\nfn one() {}\n#[allow(renamed_and_removed_lints, clippy::option_unwrap_used)]\nfn two() {}\n#[allow(clippy::map_err_ignore, unused_results, nonstandard_style)]\nfn three() {}\n#[expect(clippy::too_many_lines, reason = \"one table\")]\nfn four() {}\n";

        assert_eq!(lines_of(Rule::NoForbiddenExpects, text), [1, 3, 3, 5, 5, 5]);
    }

    #[test]
    fn nested_conditional_waivers_are_reported() {
        let text = "#[cfg_attr(all(), cfg_attr(all(), allow(clippy::unwrap_used)))]\nfn run() {}\n";

        assert_eq!(lines_of(Rule::NoCrateLevelAllow, text), [1]);
        assert_eq!(lines_of(Rule::NoForbiddenExpects, text), [1]);
    }

    #[test]
    fn a_waiver_on_a_module_is_reported() {
        let text = "#[allow(clippy::too_many_lines)]\nmod big;\n\n#[expect(clippy::cast_sign_loss, reason = \"pixels are positive\")]\nmod pixels {\n    #[expect(clippy::cast_possible_truncation, reason = \"fits in u8\")]\n    fn narrow() {}\n}\n";

        assert_eq!(lines_of(Rule::NoCrateLevelAllow, text), [1, 4]);
    }

    #[test]
    fn waivers_inside_macros_are_reported() {
        let text = "macro_rules! quiet {\n    ($item:item) => {\n        #[allow(clippy::unwrap_used, dead_code)]\n        $item\n    };\n}\n\nquiet! {\n    #[allow(clippy::too_many_lines)]\n    fn hidden() {}\n}\n";

        assert_eq!(lines_of(Rule::NoCrateLevelAllow, text), [3, 9]);
        assert_eq!(lines_of(Rule::NoForbiddenExpects, text), [3, 3]);
    }

    #[test]
    fn file_level_waivers_are_reported() {
        let text = "#![allow(clippy::too_many_lines)]\n\nfn run() {}\n";

        assert_eq!(lines_of(Rule::NoCrateLevelAllow, text), [1]);
    }

    #[test]
    fn waivers_behind_conditional_attributes_are_reported() {
        let text = "#[cfg_attr(feature = \"fast\", allow(dead_code))]\nfn run() {}\n";

        assert_eq!(lines_of(Rule::NoCrateLevelAllow, text), [1]);
        assert_eq!(lines_of(Rule::NoForbiddenExpects, text), [1]);
    }
}
