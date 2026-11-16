//! `no-abbreviations`: names you declare spell their words out.
//!
//! Descriptive, boring names: long over clever, and no abbreviation that needs a decoder.
//! Only names this code declares are checked (bindings, parameters, functions, fields,
//! types, variants, constants), never the names it uses from other crates. Methods in a
//! trait impl keep the trait's names. The list is the abbreviations that need decoding;
//! words the language itself uses, like `str`, `len`, `args` or `fmt`, aren't on it. A name
//! that's one letter behind underscores (`_x`, `f_`) is an abbreviation too; a plain one-letter
//! name is clippy's `min_ident_chars`.
//!
//! `no-ceremony-names` reads the same names for a last word that says nothing about the
//! thing: `UserManager`, `ParserFactory`, `StoreImpl`, `config_old`. `impl` counts only on a
//! type's name: in `in_trait_impl` it's Rust's `impl` block.

use proc_macro2::Ident;
use syn::visit::{self, Visit};
use syn::{
    Field, ImplItemFn, ItemConst, ItemEnum, ItemFn, ItemImpl, ItemStatic, ItemStruct, ItemTrait,
    ItemType, PatIdent, TraitItemFn, Variant,
};

use super::{FileContext, Finding, Rule};

const ABBREVIATIONS: [(&str, &str); 27] = [
    ("attr", "attribute"),
    ("btn", "button"),
    ("buf", "buffer"),
    ("cb", "callback"),
    ("cfg", "config"),
    ("cnt", "count"),
    ("ctx", "context"),
    ("cur", "current"),
    ("curr", "current"),
    ("desc", "description"),
    ("dst", "destination"),
    ("elem", "element"),
    ("err", "error"),
    ("evt", "event"),
    ("idx", "index"),
    ("mgr", "manager"),
    ("msg", "message"),
    ("num", "number"),
    ("obj", "object"),
    ("pkg", "package"),
    ("prev", "previous"),
    ("ptr", "pointer"),
    ("req", "request"),
    ("res", "result or response"),
    ("resp", "response"),
    ("src", "source"),
    ("val", "value"),
];

/// Last words that name a role in a pattern, or a leftover, instead of what the thing is.
const CEREMONY_WORDS: [&str; 7] = [
    "bak", "factory", "helper", "helpers", "impl", "manager", "old",
];

struct Abbreviations {
    findings: Vec<Finding>,
    in_trait_impl: bool,
}

impl Abbreviations {
    fn name(&mut self, ident: &Ident) {
        let name = ident.to_string();
        let line = ident.span().start().line;
        let bare = name
            .trim_start_matches("r#")
            .trim_matches('_');

        if bare.chars().count() == 1 && bare.len() < name.len() {
            self.findings.push(Finding {
                line,
                rule: Rule::NoAbbreviations,
                message: format!("`{name}` is one letter behind underscores; name what it holds"),
                edit: None,
            });
        }

        let parts = words(&name);

        if let Some(last) = parts
            .last()
            .filter(|word| CEREMONY_WORDS.contains(&word.as_str()))
            .filter(|word| *word != "impl" || name.starts_with(char::is_uppercase))
            && parts.len() > 1
        {
            self.findings.push(Finding {
                line,
                rule: Rule::NoCeremonyNames,
                message: format!("`{name}` ends in `{last}`, which says nothing about what it is; name what it holds or does"),
                edit: None,
            });
        }

        let spelled_out = parts.into_iter().find_map(|word| {
            ABBREVIATIONS
                .iter()
                .find(|(short, _)| *short == word)
                .map(|(short, long)| (*short, *long))
        });

        if let Some((short, long)) = spelled_out {
            self.findings.push(Finding {
                line,
                rule: Rule::NoAbbreviations,
                message: format!("`{name}` abbreviates `{short}`; spell it out ({long})"),
                edit: None,
            });
        }
    }
}

impl<'ast> Visit<'ast> for Abbreviations {
    fn visit_field(&mut self, field: &'ast Field) {
        if let Some(ident) = &field.ident {
            self.name(ident);
        }

        visit::visit_field(self, field);
    }

    fn visit_impl_item_fn(&mut self, method: &'ast ImplItemFn) {
        if !self.in_trait_impl {
            self.name(&method.sig.ident);
        }

        visit::visit_impl_item_fn(self, method);
    }

    fn visit_item_const(&mut self, constant: &'ast ItemConst) {
        self.name(&constant.ident);
        visit::visit_item_const(self, constant);
    }

    fn visit_item_enum(&mut self, definition: &'ast ItemEnum) {
        self.name(&definition.ident);
        visit::visit_item_enum(self, definition);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.name(&function.sig.ident);
        visit::visit_item_fn(self, function);
    }

    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        let outer = self.in_trait_impl;

        self.in_trait_impl = implementation.trait_.is_some();
        visit::visit_item_impl(self, implementation);
        self.in_trait_impl = outer;
    }

    fn visit_item_static(&mut self, constant: &'ast ItemStatic) {
        self.name(&constant.ident);
        visit::visit_item_static(self, constant);
    }

    fn visit_item_struct(&mut self, definition: &'ast ItemStruct) {
        self.name(&definition.ident);
        visit::visit_item_struct(self, definition);
    }

    fn visit_item_trait(&mut self, definition: &'ast ItemTrait) {
        self.name(&definition.ident);
        visit::visit_item_trait(self, definition);
    }

    fn visit_item_type(&mut self, alias: &'ast ItemType) {
        self.name(&alias.ident);
        visit::visit_item_type(self, alias);
    }

    fn visit_pat_ident(&mut self, binding: &'ast PatIdent) {
        self.name(&binding.ident);
        visit::visit_pat_ident(self, binding);
    }

    fn visit_trait_item_fn(&mut self, method: &'ast TraitItemFn) {
        self.name(&method.sig.ident);
        visit::visit_trait_item_fn(self, method);
    }

    fn visit_variant(&mut self, variant: &'ast Variant) {
        self.name(&variant.ident);
        visit::visit_variant(self, variant);
    }
}

/// `parse_cfg_file` and `ParseCfgFile` both give `parse`, `cfg`, `file`.
pub(super) fn words(name: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut previous_lower = false;

    for character in name.trim_start_matches("r#").chars() {
        let starts_word = character == '_' || (character.is_uppercase() && previous_lower);

        if starts_word || found.is_empty() {
            found.push(String::new());
        }

        if character != '_'
            && let Some(word) = found.last_mut()
        {
            word.extend(character.to_lowercase());
        }

        previous_lower = character.is_lowercase() || character.is_ascii_digit();
    }

    found.retain(|word| !word.is_empty());

    found
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut abbreviations = Abbreviations {
        findings: Vec::new(),
        in_trait_impl: false,
    };

    abbreviations.visit_file(file.syntax);

    abbreviations.findings
}

#[cfg(test)]
mod tests {
    use super::words;
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn words_split_snake_and_pascal_case() {
        assert_eq!(words("parse_cfg_file"), ["parse", "cfg", "file"]);
        assert_eq!(words("ParseCfgFile"), ["parse", "cfg", "file"]);
        assert_eq!(words("_unused_ctx"), ["unused", "ctx"]);
        assert_eq!(words("MAX_IDX"), ["max", "idx"]);
    }

    #[test]
    fn declared_abbreviations_are_reported() {
        let text = "struct BtnState {\n    prev_val: u8,\n}\n\nfn handle(ctx: &Context) {\n    let msg = read(ctx);\n}\n";

        assert_eq!(lines_of(Rule::NoAbbreviations, text), [1, 2, 5, 6]);
    }

    #[test]
    fn names_used_from_elsewhere_are_left_alone() {
        let text = "fn handle(context: &Context) {\n    let message = context.cfg.msg();\n}\n";

        assert_eq!(lines_of(Rule::NoAbbreviations, text), NONE);
    }

    #[test]
    fn one_letter_behind_underscores_is_reported() {
        let text = "fn main() {\n    let _x = 1;\n    let f_ = 2;\n    let _ = 3;\n    let _width = 4;\n}\n";

        assert_eq!(lines_of(Rule::NoAbbreviations, text), [2, 3]);
    }

    #[test]
    fn ceremony_last_words_are_reported() {
        let text = "struct UserManager;\nstruct ParserFactory;\nstruct StoreImpl;\n\nfn load(config_old: Config, old_config: Config, manager: Manager, in_trait_impl: bool) {}\n";

        assert_eq!(lines_of(Rule::NoCeremonyNames, text), [1, 2, 3, 5]);
    }

    #[test]
    fn trait_impl_methods_keep_the_trait_names() {
        let text = "impl Iterator for Lines {\n    fn next_ptr(&mut self) -> Option<u8> {\n        None\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoAbbreviations, text), NONE);
    }
}
