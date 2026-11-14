//! What counts as test code: `tests/`, `#[test]`-like functions and `#[cfg(test)]` modules.

use std::path::{Component, Path};

use syn::{Attribute, Meta};

/// Any attribute whose last segment is `test`: `#[test]`, `#[tokio::test]`, `#[rstest::test]`.
pub(super) fn is_test_fn(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "test")
    })
}

pub(super) fn is_test_module(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && matches!(&attribute.meta, Meta::List(list) if list.tokens.to_string() == "test")
    })
}

pub(super) fn is_test_path(path: &Path) -> bool {
    path.components()
        .any(|component| component == Component::Normal("tests".as_ref()))
}
