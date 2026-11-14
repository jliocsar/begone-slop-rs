//! `no-numbered-names` and `no-type-in-name`: names that say what a value is for.
//!
//! `item2` next to `item` is a name that wasn't thought through: say how the two differ. And
//! `user_vec` or `name_str` spends the name on what the type already says. Checked over the
//! names a function declares (parameters and bindings) and the fields of each type. Methods
//! in trait impls keep the trait's names, and conversions (`as_`, `to_`, `into_`, `from_`)
//! may name the type they convert to.

use std::collections::HashSet;

use proc_macro2::Ident;
use syn::visit::{self, Visit};
use syn::{
    Block, Fields, ImplItemFn, Item, ItemFn, ItemImpl, ItemStruct, PatIdent, Signature,
    TraitItemFn, Variant,
};

use super::abbreviations::words;
use super::{Finding, Rule};

const TYPE_WORDS: [&str; 10] = [
    "arr", "btreemap", "btreeset", "dict", "hashmap", "hashset", "obj", "str", "vec", "vector",
];

const CONVERSION_PREFIXES: [&str; 4] = ["as_", "from_", "into_", "to_"];

/// A name that asks a question about a type (`wraps_str`, `is_vec`) is a flag, not a value.
const PREDICATE_WORDS: [&str; 9] = [
    "accepts", "can", "contains", "has", "is", "needs", "should", "uses", "wraps",
];

struct Names {
    findings: Vec<Finding>,
    in_trait_impl: bool,
}

/// The names one function declares: its parameters and every binding in its body.
#[derive(Default)]
struct Declared {
    names: Vec<(String, usize)>,
}

impl Names {
    fn function(&mut self, signature: &Signature, body: Option<&Block>) {
        if !self.in_trait_impl {
            self.typed_name(&signature.ident, Some(&signature.ident.to_string()));
        }

        let mut declared = Declared::default();

        for input in &signature.inputs {
            declared.visit_fn_arg(input);
        }

        if let Some(body) = body {
            declared.visit_block(body);
        }

        self.numbered(&declared.names);

        for (name, line) in &declared.names {
            self.type_word(name, *line);
        }
    }

    fn fields(&mut self, fields: &Fields) {
        let names: Vec<(String, usize)> = fields
            .iter()
            .filter_map(|field| field.ident.as_ref())
            .map(|ident| (ident.to_string(), ident.span().start().line))
            .collect();
        self.numbered(&names);

        for (name, line) in &names {
            self.type_word(name, *line);
        }
    }

    fn numbered(&mut self, names: &[(String, usize)]) {
        let all: HashSet<&str> = names.iter().map(|(name, _)| name.as_str()).collect();
        let bases: Vec<(&str, &str, usize)> = names
            .iter()
            .filter_map(|(name, line)| Some((name.as_str(), numbered_base(name)?, *line)))
            .collect();

        for (name, base, line) in &bases {
            let has_sibling = all.contains(base)
                || bases
                    .iter()
                    .any(|(other, other_base, _)| other != name && other_base == base);

            if has_sibling {
                self.findings.push(Finding {
                    line: *line,
                    rule: Rule::NoNumberedNames,
                    message: format!("`{name}` is a numbered copy of `{base}`; name what makes this one different"),
                    edit: None,
                });
            }
        }
    }

    fn typed_name(&mut self, ident: &Ident, name: Option<&str>) {
        let name = name.map_or_else(|| ident.to_string(), str::to_owned);

        if CONVERSION_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            return;
        }

        self.type_word(&name, ident.span().start().line);
    }

    fn type_word(&mut self, name: &str, line: usize) {
        let parts = words(name);

        let predicate = parts
            .first()
            .is_some_and(|word| PREDICATE_WORDS.contains(&word.as_str()));

        if parts.len() < 2 || predicate {
            return;
        }

        if let Some(word) = parts
            .last()
            .filter(|word| TYPE_WORDS.contains(&word.as_str()))
        {
            self.findings.push(Finding {
                line,
                rule: Rule::NoTypeInName,
                message: format!("`{name}` ends in `{word}`, which the type already says; name what it holds (`users`, `sessions_by_id`)"),
                edit: None,
            });
        }
    }
}

impl<'ast> Visit<'ast> for Declared {
    fn visit_item(&mut self, _nested: &'ast Item) {}

    fn visit_pat_ident(&mut self, binding: &'ast PatIdent) {
        self.names
            .push((binding.ident.to_string(), binding.ident.span().start().line));
        visit::visit_pat_ident(self, binding);
    }
}

impl<'ast> Visit<'ast> for Names {
    fn visit_impl_item_fn(&mut self, method: &'ast ImplItemFn) {
        self.function(&method.sig, Some(&method.block));
        visit::visit_impl_item_fn(self, method);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.function(&function.sig, Some(&function.block));
        visit::visit_item_fn(self, function);
    }

    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        let outer = self.in_trait_impl;
        self.in_trait_impl = implementation.trait_.is_some();
        visit::visit_item_impl(self, implementation);
        self.in_trait_impl = outer;
    }

    fn visit_item_struct(&mut self, definition: &'ast ItemStruct) {
        self.fields(&definition.fields);
        visit::visit_item_struct(self, definition);
    }

    fn visit_trait_item_fn(&mut self, method: &'ast TraitItemFn) {
        self.function(&method.sig, method.default.as_ref());
        visit::visit_trait_item_fn(self, method);
    }

    fn visit_variant(&mut self, variant: &'ast Variant) {
        self.fields(&variant.fields);
        visit::visit_variant(self, variant);
    }
}

/// `item2` and `item_2` give `item`; `sha256` gives `sha`, which only counts when `sha`
/// (or `sha1`) is declared too.
fn numbered_base(name: &str) -> Option<&str> {
    let base = name.trim_end_matches(|character: char| character.is_ascii_digit());

    (base.len() < name.len())
        .then(|| base.trim_end_matches('_'))
        .filter(|base| !base.is_empty())
}

pub(super) fn check(file: &syn::File) -> Vec<Finding> {
    let mut names = Names {
        findings: Vec::new(),
        in_trait_impl: false,
    };
    names.visit_file(file);

    names.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn a_numbered_copy_is_reported() {
        let text = "fn merge(item: Item) {\n    let item2 = item.clone();\n    let sha256 = digest(&item2);\n}\n";

        assert_eq!(lines_of(Rule::NoNumberedNames, text), [2]);
    }

    #[test]
    fn numbered_siblings_are_reported() {
        let text = "struct Pair {\n    point1: Point,\n    point2: Point,\n}\n";

        assert_eq!(lines_of(Rule::NoNumberedNames, text), [2, 3]);
    }

    #[test]
    fn a_type_word_at_the_end_is_reported() {
        let text = "fn load(name_str: &str) {\n    let user_vec = users(name_str);\n}\n";

        assert_eq!(lines_of(Rule::NoTypeInName, text), [1, 2]);
    }

    #[test]
    fn conversions_and_single_words_pass() {
        let text = "impl Name {\n    fn as_str(&self) -> &str {\n        let vec = self.parts();\n        let wraps_str = vec.is_empty();\n        join(vec, wraps_str)\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoTypeInName, text), NONE);
    }
}
