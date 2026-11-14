//! `no-wildcard-on-local-enum`: matching an enum this file defines lists every variant.
//!
//! A `_ =>` arm (or a catch-all binding) swallows the variant someone adds next, and the
//! compiler stays quiet. For an enum from another crate that's often required (it may be
//! `#[non_exhaustive]`), which is why clippy's `wildcard_enum_match_arm` is too noisy to turn
//! on. For an enum defined in the same file, there's no excuse. Guarded catch-alls
//! (`_ if ... =>`) don't count, and neither do wildcards nested inside a tuple.

use std::collections::HashSet;

use syn::visit::{self, Visit};
use syn::{ExprMatch, File, ItemEnum, ItemImpl, Pat, Path, Type};

use super::{Finding, Rule};

struct Exhaustive {
    enums: HashSet<String>,
    impl_self: Option<String>,
    findings: Vec<Finding>,
}

#[derive(Default)]
struct LocalEnums {
    names: HashSet<String>,
}

impl Exhaustive {
    /// The enum a pattern names: `Kind::Use` gives `Kind`, `Self::Use` the impl's type.
    fn enum_of(&self, path: &Path) -> Option<String> {
        let mut segments = path.segments.iter().rev();
        segments.next()?;
        let owner = segments.next()?.ident.to_string();

        if owner == "Self" {
            self.impl_self.clone()
        } else {
            Some(owner)
        }
    }

    fn pattern_enum(&self, pattern: &Pat) -> Option<String> {
        match pattern {
            Pat::Path(path) => self.enum_of(&path.path),
            Pat::TupleStruct(tuple) => self.enum_of(&tuple.path),
            Pat::Struct(structure) => self.enum_of(&structure.path),
            Pat::Or(alternatives) => alternatives
                .cases
                .iter()
                .find_map(|case| self.pattern_enum(case)),
            _ => None,
        }
    }
}

impl<'ast> Visit<'ast> for Exhaustive {
    fn visit_expr_match(&mut self, matching: &'ast ExprMatch) {
        let local = matching
            .arms
            .iter()
            .find_map(|arm| self.pattern_enum(&arm.pat))
            .filter(|name| self.enums.contains(name));

        if let Some(name) = local {
            for arm in matching
                .arms
                .iter()
                .filter(|arm| arm.guard.is_none() && is_catch_all(&arm.pat))
            {
                self.findings.push(Finding {
                    line: arm.fat_arrow_token.spans[0].start().line,
                    rule: Rule::NoWildcardOnLocalEnum,
                    message: format!(
                        "this catch-all hides the next variant added to `{name}`, which this file defines; list the variants so the compiler flags a new one"
                    ),
                    edit: None,
                });
            }
        }

        visit::visit_expr_match(self, matching);
    }

    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        let outer = self.impl_self.take();

        self.impl_self = match &*implementation.self_ty {
            Type::Path(path) => path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string()),
            _ => None,
        };

        visit::visit_item_impl(self, implementation);
        self.impl_self = outer;
    }
}

impl<'ast> Visit<'ast> for LocalEnums {
    fn visit_item_enum(&mut self, definition: &'ast ItemEnum) {
        self.names.insert(definition.ident.to_string());
        visit::visit_item_enum(self, definition);
    }
}

/// `_`, or a bare lowercase binding like `other`.
fn is_catch_all(pattern: &Pat) -> bool {
    match pattern {
        Pat::Wild(_) => true,
        Pat::Ident(binding) => {
            binding.subpat.is_none()
                && binding
                    .ident
                    .to_string()
                    .starts_with(|character: char| character.is_lowercase() || character == '_')
        }
        _ => false,
    }
}

pub(super) fn check(file: &File) -> Vec<Finding> {
    let mut local = LocalEnums::default();
    local.visit_file(file);
    let mut exhaustive = Exhaustive {
        enums: local.names,
        impl_self: None,
        findings: Vec::new(),
    };
    exhaustive.visit_file(file);

    exhaustive.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn a_wildcard_on_a_local_enum_is_reported() {
        let text = "enum Mode {\n    Fast,\n    Slow,\n    Off,\n}\n\nfn speed(mode: Mode) -> u8 {\n    match mode {\n        Mode::Fast => 3,\n        _ => 0,\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoWildcardOnLocalEnum, text), [10]);
    }

    #[test]
    fn self_paths_resolve_to_the_impl_type() {
        let text = "enum Mode {\n    Fast,\n    Slow,\n}\n\nimpl Mode {\n    fn fast(self) -> bool {\n        match self {\n            Self::Fast => true,\n            other => false,\n        }\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoWildcardOnLocalEnum, text), [10]);
    }

    #[test]
    fn foreign_enums_may_use_a_wildcard() {
        let text = "fn kind(error: &io::Error) -> u8 {\n    match error.kind() {\n        io::ErrorKind::NotFound => 1,\n        _ => 0,\n    }\n}\n";

        assert_eq!(lines_of(Rule::NoWildcardOnLocalEnum, text), NONE);
    }
}
