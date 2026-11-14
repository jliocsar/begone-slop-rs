//! What counts as test code inside a file: `#[test]`-like functions and `#[cfg(test)]`
//! modules. Whole files are test code by their [`Role`](super::Role).

use syn::Attribute;

use crate::module_paths;

/// Whether an item opens test code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    Test,
    Plain,
}

impl Scope {
    pub(super) fn of_fn(attributes: &[Attribute]) -> Self {
        if is_test_fn(attributes) {
            Self::Test
        } else {
            Self::Plain
        }
    }

    pub(super) fn of_module(attributes: &[Attribute]) -> Self {
        if is_test_module(attributes) {
            Self::Test
        } else {
            Self::Plain
        }
    }

    pub(super) fn depth(self) -> usize {
        usize::from(self == Self::Test)
    }
}

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

/// `#[cfg(test)]`, or `test` inside `cfg(all(...))`.
pub(super) fn is_test_module(attributes: &[Attribute]) -> bool {
    module_paths::is_test_only(attributes)
}
