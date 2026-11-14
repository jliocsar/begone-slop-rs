//! `no-boolean-trap`, `newtype-ids` and `no-stringly-fields`: signatures that say what their
//! values mean.
//!
//! `draw(true)` says nothing at the call site; `draw(Fill::Solid)` does. And an id typed as a
//! `String` or a number takes any other id of the same type without complaint; a newtype per
//! kind of id makes swapping two of them a compile error. Methods in trait impls keep the
//! trait's signature, test code is left alone, and an id newtype's own impl may take the raw
//! value it wraps.

use syn::visit::{self, Visit};
use syn::{
    Field, FnArg, GenericArgument, ImplItemFn, ItemFn, ItemImpl, ItemMod, Pat, PathArguments,
    Signature, TraitItemFn, Type,
};

use super::test_scope::{is_test_fn, is_test_module};
use super::{Context, Finding, Rule};

const RAW_ID_TYPES: [&str; 17] = [
    "String", "str", "Uuid", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64",
    "i128", "isize", "Box", "Arc",
];

const ID_COLLECTIONS: [&str; 4] = ["Vec", "HashSet", "BTreeSet", "Option"];

/// Names that hold one of a fixed set of values, which is what an enum is for.
const CATEGORY_NAMES: [&str; 12] = [
    "category", "kind", "level", "mode", "phase", "priority", "role", "severity", "stage", "state",
    "status", "variant",
];

const STRING_TYPES: [&str; 2] = ["String", "str"];
const PARSE_PREFIXES: [&str; 2] = ["from_", "parse"];

struct Signatures {
    findings: Vec<Finding>,
    test_file: bool,
    test_depth: usize,
    in_trait_impl: bool,
    in_id_impl: bool,
}

impl Signatures {
    fn skipped(&self) -> bool {
        self.test_file || self.test_depth > 0
    }

    fn signature(&mut self, signature: &Signature) {
        if self.skipped() || self.in_trait_impl {
            return;
        }

        for input in &signature.inputs {
            let FnArg::Typed(typed) = input else {
                continue;
            };

            let Pat::Ident(binding) = &*typed.pat else {
                continue;
            };

            let name = binding.ident.to_string();

            if is_plain(&typed.ty, "bool") {
                self.push(
                    binding.ident.span().start().line,
                    Rule::NoBooleanTrap,
                    format!("`{name}: bool` reads as a bare `true`/`false` at the call site; take a two-variant enum that names both cases"),
                );
            }

            let parses = PARSE_PREFIXES
                .iter()
                .any(|prefix| signature.ident.to_string().starts_with(prefix));

            if !parses {
                self.category(&name, &typed.ty, binding.ident.span().start().line);
            }

            if !self.in_id_impl {
                self.id(&name, &typed.ty, binding.ident.span().start().line);
            }
        }
    }

    fn category(&mut self, name: &str, ty: &Type, line: usize) {
        let last_word = name.rsplit('_').next().unwrap_or(name);
        let stringly = CATEGORY_NAMES.contains(&last_word) && is_string(ty);

        if stringly {
            self.push(
                line,
                Rule::NoStringlyFields,
                format!("`{name}` holds one of a fixed set of values as a string; make it an enum and parse it once, at the boundary"),
            );
        }
    }

    fn id(&mut self, name: &str, ty: &Type, line: usize) {
        let single = name == "id" || name.ends_with("_id");
        let plural = name == "ids" || name.ends_with("_ids");
        let raw = (single && is_raw_id(ty)) || (plural && collected(ty).is_some_and(is_raw_id));

        if raw {
            self.push(
                line,
                Rule::NewtypeIds,
                format!("`{name}` holds an id as a raw type, so any other id fits too; give this kind of id its own newtype"),
            );
        }
    }

    fn push(&mut self, line: usize, rule: Rule, message: String) {
        self.findings.push(Finding {
            line,
            rule,
            message,
            edit: None,
        });
    }
}

impl<'ast> Visit<'ast> for Signatures {
    fn visit_field(&mut self, field: &'ast Field) {
        if let Some(ident) = &field.ident
            && !self.skipped()
        {
            self.id(&ident.to_string(), &field.ty, ident.span().start().line);
            self.category(&ident.to_string(), &field.ty, ident.span().start().line);
        }

        visit::visit_field(self, field);
    }

    fn visit_impl_item_fn(&mut self, method: &'ast ImplItemFn) {
        self.signature(&method.sig);
        visit::visit_impl_item_fn(self, method);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let is_test = is_test_fn(&function.attrs);
        self.test_depth += usize::from(is_test);
        self.signature(&function.sig);
        visit::visit_item_fn(self, function);
        self.test_depth -= usize::from(is_test);
    }

    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        let outer = (self.in_trait_impl, self.in_id_impl);
        self.in_trait_impl = implementation.trait_.is_some();
        self.in_id_impl = matches!(&*implementation.self_ty, Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident.to_string().ends_with("Id")));
        visit::visit_item_impl(self, implementation);
        (self.in_trait_impl, self.in_id_impl) = outer;
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        let is_test = is_test_module(&module.attrs);
        self.test_depth += usize::from(is_test);
        visit::visit_item_mod(self, module);
        self.test_depth -= usize::from(is_test);
    }

    fn visit_trait_item_fn(&mut self, method: &'ast TraitItemFn) {
        self.signature(&method.sig);
        visit::visit_trait_item_fn(self, method);
    }
}

/// The type's last path segment is `name`, with no generics: `bool`, `String`.
fn is_plain(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(path) if path.qself.is_none()
        && path.path.segments.last().is_some_and(|segment| segment.ident == name && segment.arguments.is_empty()))
}

/// `String`, `&str`, `Box<str>`, `Arc<str>`, `Cow<str>`.
fn is_string(ty: &Type) -> bool {
    match ty {
        Type::Reference(reference) => is_string(&reference.elem),
        Type::Path(path) => path.path.segments.last().is_some_and(|segment| {
            let wraps_str = matches!(&segment.arguments, PathArguments::AngleBracketed(arguments)
                if arguments.args.iter().any(|argument| matches!(argument, GenericArgument::Type(inner) if is_plain(inner, "str"))));

            STRING_TYPES.iter().any(|name| segment.ident == name) || wraps_str
        }),
        _ => false,
    }
}

/// `String`, `&str`, an integer, `Uuid`, `Box<str>`, `Arc<str>`.
fn is_raw_id(ty: &Type) -> bool {
    match ty {
        Type::Reference(reference) => is_raw_id(&reference.elem),
        Type::Path(path) => path.path.segments.last().is_some_and(|segment| {
            let wraps_str = matches!(&segment.arguments, PathArguments::AngleBracketed(arguments)
                if arguments.args.iter().any(|argument| matches!(argument, GenericArgument::Type(inner) if is_plain(inner, "str"))));

            RAW_ID_TYPES.iter().any(|raw| segment.ident == raw)
                && (segment.arguments.is_empty() || wraps_str)
        }),
        _ => false,
    }
}

/// The element type of `Vec<T>`, `HashSet<T>`, `&[T]` and the like.
fn collected(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Reference(reference) => collected(&reference.elem),
        Type::Slice(slice) => Some(&slice.elem),
        Type::Path(path) => {
            let segment = path.path.segments.last()?;

            let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                return None;
            };

            ID_COLLECTIONS
                .iter()
                .any(|collection| segment.ident == collection)
                .then(|| {
                    arguments.args.iter().find_map(|argument| match argument {
                        GenericArgument::Type(inner) => Some(inner),
                        _ => None,
                    })
                })
                .flatten()
        }
        _ => None,
    }
}

pub(super) fn check(file: &syn::File, context: &Context) -> Vec<Finding> {
    let mut signatures = Signatures {
        findings: Vec::new(),
        test_file: context.in_tests(),
        test_depth: 0,
        in_trait_impl: false,
        in_id_impl: false,
    };
    signatures.visit_file(file);

    signatures.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn a_stringly_category_is_reported() {
        let text = "struct Job {\n    status: String,\n    log_level: &'static str,\n    name: String,\n}\n\nfn parse_mode(mode: &str) -> Mode {\n    lookup(mode)\n}\n";

        assert_eq!(lines_of(Rule::NoStringlyFields, text), [2, 3]);
    }

    #[test]
    fn a_bool_parameter_is_reported() {
        let text = "fn draw(shape: &Shape, filled: bool) {}\n";

        assert_eq!(lines_of(Rule::NoBooleanTrap, text), [1]);
    }

    #[test]
    fn trait_impls_keep_their_signature() {
        let text = "impl Toggle for Lamp {\n    fn set(&mut self, on: bool) {}\n}\n";

        assert_eq!(lines_of(Rule::NoBooleanTrap, text), NONE);
    }

    #[test]
    fn raw_ids_are_reported() {
        let text = "struct Session {\n    session_id: String,\n    owner_ids: Vec<u64>,\n    name: String,\n}\n\nfn find(id: &str) {}\n";

        assert_eq!(lines_of(Rule::NewtypeIds, text), [2, 3, 7]);
    }

    #[test]
    fn newtyped_ids_pass() {
        let text = "struct Session {\n    session_id: SessionId,\n}\n\nimpl SessionId {\n    fn new(id: String) -> Self {\n        Self(id)\n    }\n}\n";

        assert_eq!(lines_of(Rule::NewtypeIds, text), NONE);
    }
}
