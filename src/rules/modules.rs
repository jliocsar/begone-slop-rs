//! `no-junk-drawer-modules` and `module-doc`: every module has a job and says what it is.
//!
//! A `utils` or `helpers` module collects code that has no home yet; give each piece one where
//! it's used. And each file opens with a `//!` comment saying what it's for, the line a reader
//! (or an agent) needs before reading the rest. Build scripts are exempt from the doc rule.

use std::path::Component;

use syn::{AttrStyle, File};

use super::{Finding, Rule};
use crate::source::Source;

const JUNK_DRAWERS: [&str; 7] = [
    "common", "helper", "helpers", "misc", "stuff", "util", "utils",
];

pub(super) fn check(source: &Source, file: &File) -> Vec<Finding> {
    let drawer =
        source
            .path
            .with_extension("")
            .components()
            .find_map(|component| match component {
                Component::Normal(part) => part
                    .to_str()
                    .filter(|part| JUNK_DRAWERS.contains(part))
                    .map(str::to_owned),
                _ => None,
            });
    let documented = file.attrs.iter().any(|attribute| {
        matches!(attribute.style, AttrStyle::Inner(_)) && attribute.path().is_ident("doc")
    });
    let build_script = source
        .path
        .file_name()
        .is_some_and(|name| name == "build.rs");
    let mut findings = Vec::new();

    if let Some(drawer) = drawer {
        findings.push(Finding {
            line: 1,
            rule: Rule::NoJunkDrawerModules,
            message: format!("`{drawer}` is a junk drawer, not a job; move each piece next to the code that uses it, or into a module named for what it does"),
            edit: None,
        });
    }

    if !documented && !build_script {
        findings.push(Finding {
            line: 1,
            rule: Rule::ModuleDoc,
            message: "this file doesn't say what it's for; open it with a `//!` comment".to_owned(),
            edit: None,
        });
    }

    findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::findings;

    fn rules_in(path: &str, text: &str) -> Vec<Rule> {
        findings(path, text)
            .into_iter()
            .map(|finding| finding.rule)
            .collect()
    }

    #[test]
    fn a_utils_module_is_reported() {
        assert!(
            rules_in("src/utils.rs", "//! Odds and ends.\n").contains(&Rule::NoJunkDrawerModules)
        );
        assert!(
            rules_in("src/helpers/strings.rs", "//! Strings.\n")
                .contains(&Rule::NoJunkDrawerModules)
        );
    }

    #[test]
    fn an_undocumented_file_is_reported() {
        assert!(rules_in("src/session.rs", "fn open() {}\n").contains(&Rule::ModuleDoc));
        assert!(
            !rules_in("src/session.rs", "//! Sessions.\n\nfn open() {}\n")
                .contains(&Rule::ModuleDoc)
        );
    }
}
