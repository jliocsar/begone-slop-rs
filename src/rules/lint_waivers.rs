//! `no-forbidden-expects` and `no-crate-level-allow`: waivers a reason can't buy.
//!
//! Some lints guard the gate itself: waive `unwrap_used` or `dead_code` and the slop they catch
//! walks back in with a polite reason attached. Outside tests, those lints can't be waived at
//! all. And no lint is waived for a whole file or crate (`#![allow]`, `#![expect]`), or behind
//! a `cfg_attr`, where a waiver hides from review.

use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{AttrStyle, Attribute, ItemFn, ItemMod, Meta, MetaList, Token};

use super::paths::segments;
use super::test_scope::{is_test_fn, is_test_module, is_test_path};
use super::{Finding, Rule};
use crate::source::Source;

const FORBIDDEN: [&str; 20] = [
    "clippy::all",
    "clippy::allow_attributes",
    "clippy::allow_attributes_without_reason",
    "clippy::dbg_macro",
    "clippy::expect_used",
    "clippy::panic",
    "clippy::pedantic",
    "clippy::todo",
    "clippy::undocumented_unsafe_blocks",
    "clippy::unimplemented",
    "clippy::unwrap_used",
    "dead_code",
    "unsafe_code",
    "unused",
    "unused_imports",
    "unused_must_use",
    "unused_mut",
    "unused_variables",
    "warnings",
    "clippy::unused_result_ok",
];

const LEVELS: [&str; 2] = ["allow", "expect"];

struct LintWaivers {
    findings: Vec<Finding>,
    test_file: bool,
    test_depth: usize,
}

impl LintWaivers {
    fn waived_lints(list: &MetaList) -> Vec<String> {
        list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .map(|metas| {
                metas
                    .iter()
                    .filter_map(|meta| match meta {
                        Meta::Path(path) => Some(segments(path).join("::")),
                        Meta::List(_) | Meta::NameValue(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The `allow`/`expect` lists inside `cfg_attr(condition, ...)`.
    fn conditional_levels(list: &MetaList) -> Vec<MetaList> {
        list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .map(|metas| {
                metas
                    .into_iter()
                    .skip(1)
                    .filter_map(|meta| match meta {
                        Meta::List(inner)
                            if LEVELS.iter().any(|level| inner.path.is_ident(level)) =>
                        {
                            Some(inner)
                        }
                        Meta::Path(_) | Meta::List(_) | Meta::NameValue(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn push(&mut self, attribute: &Attribute, rule: Rule, message: String) {
        self.findings.push(Finding {
            line: attribute.pound_token.span.start().line,
            rule,
            message,
            edit: None,
        });
    }

    fn check_levels(&mut self, attribute: &Attribute, list: &MetaList) {
        if self.test_file || self.test_depth > 0 {
            return;
        }

        for lint in Self::waived_lints(list) {
            if FORBIDDEN.contains(&lint.as_str()) {
                self.push(
                    attribute,
                    Rule::NoForbiddenExpects,
                    format!("`{lint}` can't be waived outside tests, not even with a reason; fix what it reports"),
                );
            }
        }
    }
}

impl<'ast> Visit<'ast> for LintWaivers {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        let Meta::List(list) = &attribute.meta else {
            return;
        };

        let is_level = LEVELS.iter().any(|level| list.path.is_ident(level));

        if is_level {
            if matches!(attribute.style, AttrStyle::Inner(_)) {
                self.push(
                    attribute,
                    Rule::NoCrateLevelAllow,
                    "a waiver for a whole file or module hides every case it covers; put `#[expect(lint, reason = \"...\")]` on the item that needs it".to_owned(),
                );
            }

            self.check_levels(attribute, list);
        } else if list.path.is_ident("cfg_attr") {
            for inner in Self::conditional_levels(list) {
                self.push(
                    attribute,
                    Rule::NoCrateLevelAllow,
                    "a waiver behind `cfg_attr` only applies in some builds and hides from review; waive the item directly, or not at all".to_owned(),
                );
                self.check_levels(attribute, &inner);
            }
        }
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let is_test = is_test_fn(&function.attrs);
        self.test_depth += usize::from(is_test);
        visit::visit_item_fn(self, function);
        self.test_depth -= usize::from(is_test);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        let is_test = is_test_module(&module.attrs);
        self.test_depth += usize::from(is_test);
        visit::visit_item_mod(self, module);
        self.test_depth -= usize::from(is_test);
    }
}

pub(super) fn check(source: &Source, file: &syn::File) -> Vec<Finding> {
    let mut waivers = LintWaivers {
        findings: Vec::new(),
        test_file: is_test_path(&source.path),
        test_depth: 0,
    };
    waivers.visit_file(file);

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
