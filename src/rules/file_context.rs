//! Everything the rules know about one file, worked out once before any rule runs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::paths::Imports;
use super::test_scope::TestLines;
use super::waivers::{self, Waiver};
use super::{Context, Role};
use crate::config::Config;
use crate::source::Source;

/// One file as the rules see it: its text, its syntax tree, where it sits in the project, and
/// the facts several rules share (which lines are tests, what each import names, which
/// `bs-allow` waivers it holds).
pub struct FileContext<'run> {
    pub(super) source: &'run Source,
    pub(super) syntax: &'run syn::File,
    pub(super) config: &'run Config,
    pub(super) role: Role,
    pub(super) absolute: &'run Path,
    pub(super) relative: Option<&'run Path>,
    pub(super) listed: &'run HashSet<PathBuf>,
    pub(super) imports: Imports,
    pub(super) tests: TestLines,
    pub(super) waivers: Vec<Waiver>,
}

impl<'run> FileContext<'run> {
    /// Reads what every rule needs from `syntax` once, with the run's facts about the file.
    #[must_use]
    pub fn new(
        source: &'run Source,
        syntax: &'run syn::File,
        context: &'run Context<'run>,
    ) -> Self {
        Self {
            source,
            syntax,
            config: context.config,
            role: context.role,
            absolute: &context.absolute,
            relative: context.relative.as_deref(),
            listed: context.listed,
            imports: Imports::of(syntax),
            tests: TestLines::of(syntax, context.role),
            waivers: waivers::parse(source),
        }
    }

    /// Whether `line` is test code: the whole file is, or a test function or module holds it.
    #[must_use]
    pub fn is_test(&self, line: usize) -> bool {
        self.tests.contains(line)
    }

    /// Whether the whole file is test code.
    #[must_use]
    pub fn in_tests(&self) -> bool {
        self.role.is_test()
    }
}
