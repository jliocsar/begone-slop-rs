//! The lints rustc and clippy know, and the level a `[lints]` table leaves each one at.
//!
//! Cargo turns a `[lints]` table into command-line flags sorted by priority, and rustc lets a
//! later flag override an earlier one, a group's flag setting every lint in the group. So a
//! lint ends up at the level of the highest-priority entry that names it or one of its groups.
//! When two such entries share that priority, which one comes last is Cargo's choice, so this
//! takes the looser one: the gate fails closed.

mod known;

use std::collections::HashMap;
use std::sync::LazyLock;

use toml::{Table, Value};

/// Old names that rustc and clippy still accept, with the lint each one stands for now.
const RENAMED: [(&str, &str); 39] = [
    (
        "clippy::almost_complete_letter_range",
        "clippy::almost_complete_range",
    ),
    ("clippy::blacklisted_name", "clippy::disallowed_names"),
    (
        "clippy::block_in_if_condition_expr",
        "clippy::blocks_in_conditions",
    ),
    (
        "clippy::block_in_if_condition_stmt",
        "clippy::blocks_in_conditions",
    ),
    (
        "clippy::blocks_in_if_conditions",
        "clippy::blocks_in_conditions",
    ),
    ("clippy::box_vec", "clippy::box_collection"),
    (
        "clippy::const_static_lifetime",
        "clippy::redundant_static_lifetimes",
    ),
    (
        "clippy::cyclomatic_complexity",
        "clippy::cognitive_complexity",
    ),
    (
        "clippy::derive_hash_xor_eq",
        "clippy::derived_hash_with_manual_eq",
    ),
    ("clippy::disallowed_method", "clippy::disallowed_methods"),
    ("clippy::disallowed_type", "clippy::disallowed_types"),
    ("clippy::drop_bounds", "drop_bounds"),
    (
        "clippy::eval_order_dependence",
        "clippy::mixed_read_write_in_expression",
    ),
    ("clippy::identity_conversion", "clippy::useless_conversion"),
    ("clippy::if_let_some_result", "clippy::match_result_ok"),
    ("clippy::into_iter_on_array", "array_into_iter"),
    ("clippy::invalid_ref", "invalid_value"),
    ("clippy::let_underscore_drop", "let_underscore_drop"),
    ("clippy::logic_bug", "clippy::overly_complex_bool_expr"),
    (
        "clippy::mem_discriminant_non_enum",
        "enum_intrinsics_non_enums",
    ),
    (
        "clippy::new_without_default_derive",
        "clippy::new_without_default",
    ),
    (
        "clippy::option_and_then_some",
        "clippy::bind_instead_of_map",
    ),
    ("clippy::option_expect_used", "clippy::expect_used"),
    ("clippy::option_map_unwrap_or", "clippy::map_unwrap_or"),
    ("clippy::option_map_unwrap_or_else", "clippy::map_unwrap_or"),
    ("clippy::option_unwrap_used", "clippy::unwrap_used"),
    ("clippy::panic_params", "non_fmt_panics"),
    ("clippy::ref_in_deref", "clippy::needless_borrow"),
    ("clippy::result_expect_used", "clippy::expect_used"),
    ("clippy::result_map_unwrap_or_else", "clippy::map_unwrap_or"),
    ("clippy::result_unwrap_used", "clippy::unwrap_used"),
    (
        "clippy::single_char_push_str",
        "clippy::single_char_add_str",
    ),
    ("clippy::stutter", "clippy::module_name_repetitions"),
    (
        "clippy::to_string_in_display",
        "clippy::recursive_format_impl",
    ),
    ("clippy::unknown_clippy_lints", "unknown_lints"),
    ("clippy::unused_label", "unused_labels"),
    (
        "clippy::unwrap_or_else_default",
        "clippy::unwrap_or_default",
    ),
    ("clippy::zero_width_space", "clippy::invisible_characters"),
    ("unused_doc_comment", "unused_doc_comments"),
];

/// The `[lints]` tools the preset covers, with the prefix their lints are named by.
const TOOLS: [(&str, &str); 2] = [("clippy", "clippy::"), ("rust", "")];

/// The lint that turns every warning on or off. It isn't a group of named lints, so it's
/// handled on its own.
pub const WARNINGS: &str = "warnings";

static DEFAULTS: LazyLock<HashMap<&'static str, Level>> =
    LazyLock::new(|| known::LINTS.iter().copied().collect());

static GROUPS: LazyLock<HashMap<&'static str, &'static [&'static str]>> =
    LazyLock::new(|| known::GROUPS.iter().copied().collect());

/// A lint level, loosest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Allow,
    Warn,
    Deny,
    Forbid,
}

/// One entry of a `[lints]` table, by its full name (`clippy::all`, `dead_code`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub level: Level,
    pub priority: i64,
}

/// The level a table leaves a lint at, and the entry that set it. No entry means the lint's
/// default; no level means a name rustc and clippy don't know.
#[derive(Clone, Copy, Debug)]
pub struct Resolved<'table> {
    pub level: Option<Level>,
    pub by: Option<&'table Entry>,
}

impl Level {
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "allow" => Some(Self::Allow),
            "warn" => Some(Self::Warn),
            "deny" => Some(Self::Deny),
            "forbid" => Some(Self::Forbid),
            _ => None,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Warn => "warn",
            Self::Deny => "deny",
            Self::Forbid => "forbid",
        }
    }
}

impl Entry {
    /// Whether this entry sets `lint`, by name or through a group.
    #[must_use]
    pub fn covers(&self, lint: &str) -> bool {
        self.name == lint || members(&self.name).contains(&lint)
    }
}

/// The level `lint` has when nothing sets it, or `None` for a name nobody knows.
#[must_use]
pub fn default_level(lint: &str) -> Option<Level> {
    DEFAULTS.get(lint).copied()
}

/// The lints in a group; empty for anything that isn't one.
#[must_use]
pub fn members(group: &str) -> &'static [&'static str] {
    GROUPS.get(group).copied().unwrap_or_default()
}

/// Whether `name` is a lint group, `warnings` included.
#[must_use]
pub fn is_group(name: &str) -> bool {
    name == WARNINGS || GROUPS.contains_key(name)
}

/// Every lint rustc and clippy know, by full name.
pub fn known() -> impl Iterator<Item = &'static str> {
    known::LINTS.iter().map(|(name, _)| *name)
}

/// The current name of a lint written under an old one; any other name as it is.
#[must_use]
pub fn current_name(lint: &str) -> &str {
    RENAMED
        .iter()
        .find(|(old, _)| *old == lint)
        .map_or(lint, |(_, new)| new)
}

/// A `[lints]` table's entries (`rust` and `clippy`; other tools aren't the preset's), with
/// old names under their current ones. An entry that isn't a level or `{ level, priority }`
/// comes back as an error, by its full name.
#[must_use]
pub fn entries(lints: &Table) -> (Vec<Entry>, Vec<String>) {
    let mut found = Vec::new();
    let mut unreadable = Vec::new();

    for (tool, table) in lints {
        let Some((_, prefix)) = TOOLS.iter().find(|(name, _)| name == tool) else {
            continue;
        };

        for (name, value) in table.as_table().into_iter().flatten() {
            let lint = name.replace('-', "_");
            let full = format!("{prefix}{lint}");

            match level_and_priority(value) {
                Some((level, priority)) => found.push(Entry {
                    name: current_name(&full).to_owned(),
                    level,
                    priority,
                }),
                None => unreadable.push(full),
            }
        }
    }

    (found, unreadable)
}

/// Where `lint` ends up under `entries`.
#[must_use]
pub fn resolve<'table>(entries: &'table [Entry], lint: &str) -> Resolved<'table> {
    let covering = || entries.iter().filter(|entry| entry.covers(lint));

    let Some(top) = covering().map(|entry| entry.priority).max() else {
        return Resolved {
            level: default_level(lint),
            by: None,
        };
    };

    let winner = covering()
        .filter(|entry| entry.priority == top)
        .min_by_key(|entry| entry.level);

    Resolved {
        level: winner.map(|entry| entry.level),
        by: winner,
    }
}

/// `"warn"` or `{ level = "warn", priority = -1 }`.
fn level_and_priority(value: &Value) -> Option<(Level, i64)> {
    match value {
        Value::String(level) => Some((Level::parse(level)?, 0)),
        Value::Table(table) => {
            let level = Level::parse(table.get("level")?.as_str()?)?;

            let priority = match table.get("priority") {
                Some(priority) => priority.as_integer()?,
                None => 0,
            };

            Some((level, priority))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use toml::Table;

    use super::{Level, current_name, default_level, entries, is_group, members, resolve};

    fn resolved(table: &str, lint: &str) -> Option<Level> {
        let (found, unreadable) = entries(&table.parse::<Table>().unwrap());

        assert!(unreadable.is_empty(), "{unreadable:?}");

        resolve(&found, lint).level
    }

    #[test]
    fn the_table_knows_rustc_and_clippy_lints() {
        assert_eq!(default_level("dead_code"), Some(Level::Warn));
        assert_eq!(default_level("clippy::unwrap_used"), Some(Level::Allow));
        assert_eq!(default_level("clippy::approx_constant"), Some(Level::Deny));
        assert!(members("clippy::pedantic").contains(&"clippy::too_many_lines"));
        assert!(is_group("warnings"));
        assert!(!is_group("clippy::unwrap_used"));
    }

    #[test]
    fn a_higher_priority_wins() {
        let table = "[clippy]\npedantic = { level = \"warn\", priority = -1 }\ntoo_many_lines = \"allow\"\n";

        assert_eq!(
            resolved(table, "clippy::too_many_lines"),
            Some(Level::Allow)
        );
        assert_eq!(resolved(table, "clippy::redundant_else"), Some(Level::Warn));
    }

    #[test]
    fn a_group_above_a_lint_overrides_it() {
        let table =
            "[clippy]\nunwrap_used = \"warn\"\nrestriction = { level = \"allow\", priority = 1 }\n";

        assert_eq!(resolved(table, "clippy::unwrap_used"), Some(Level::Allow));
    }

    #[test]
    fn a_tie_reads_as_the_looser_level() {
        let table = "[clippy]\nunwrap_used = \"warn\"\nrestriction = \"allow\"\n";

        assert_eq!(resolved(table, "clippy::unwrap_used"), Some(Level::Allow));
    }

    #[test]
    fn unset_lints_keep_their_default() {
        assert_eq!(
            resolved("[clippy]\n", "clippy::approx_constant"),
            Some(Level::Deny)
        );
        assert_eq!(resolved("[clippy]\n", "clippy::no_such_lint"), None);
    }

    #[test]
    fn old_names_resolve_to_the_current_lint() {
        assert_eq!(
            current_name("clippy::option_unwrap_used"),
            "clippy::unwrap_used"
        );
        assert_eq!(
            resolved(
                "[clippy]\nstutter = \"allow\"\n",
                "clippy::module_name_repetitions"
            ),
            Some(Level::Allow)
        );
    }
}
