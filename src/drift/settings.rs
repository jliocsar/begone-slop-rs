//! `clippy.toml` settings, by which way is stricter.
//!
//! A threshold may go down, a list of exemptions may shrink, a list of bans may grow, and an
//! `allow-*` switch may turn off. Each setting is compared as clippy reads it: the value the
//! file sets, or clippy's default when it sets none. A key `cargo bs` can't place is reported,
//! since it can't tell whether that key loosens anything.

use toml::{Table, Value};

/// Thresholds where a lower number reports more, with clippy's defaults.
const LOWER_IS_STRICTER: [(&str, i64); 21] = [
    ("absolute-paths-max-segments", 2),
    ("array-size-threshold", 16_384),
    ("cognitive-complexity-threshold", 25),
    ("enum-variant-name-threshold", 3),
    ("enum-variant-size-threshold", 200),
    ("excessive-nesting-threshold", 0),
    ("future-size-threshold", 16_384),
    ("large-error-threshold", 128),
    ("literal-representation-threshold", 16_384),
    ("max-fn-params-bools", 3),
    ("max-include-file-size", 1_000_000),
    ("max-struct-bools", 3),
    ("max-trait-bounds", 3),
    ("pass-by-value-size-limit", 256),
    ("single-char-binding-names-threshold", 4),
    ("stack-size-threshold", 512_000),
    ("struct-field-name-threshold", 3),
    ("too-many-arguments-threshold", 7),
    ("too-many-lines-threshold", 100),
    ("type-complexity-threshold", 250),
    ("verbose-bit-mask-threshold", 1),
];

/// Thresholds where a higher number reports more, with clippy's defaults.
const HIGHER_IS_STRICTER: [(&str, i64); 6] = [
    ("max-suggested-slice-pattern-length", 3),
    ("min-ident-chars-threshold", 1),
    ("too-large-for-stack", 200),
    ("trivial-copy-size-limit", 8),
    ("unnecessary-box-size", 128),
    ("vec-box-size-threshold", 4_096),
];

/// A threshold of zero turns its lint off rather than making it strictest.
const ZERO_IS_OFF: [&str; 1] = ["excessive-nesting-threshold"];

/// Switches that let more through when `true`, and are `true` unless set.
const LOOSE_BY_DEFAULT: [&str; 8] = [
    "accept-comment-above-attributes",
    "accept-comment-above-statement",
    "allow-comparison-to-zero",
    "allow-exact-repetitions",
    "allow-expect-in-consts",
    "allow-mixed-uninlined-format-args",
    "allow-unwrap-in-consts",
    "avoid-breaking-exported-api",
];

/// Switches that report more when `true`, and are `true` unless set.
const STRICT_BY_DEFAULT: [&str; 2] = [
    "recursive-self-in-type-definitions",
    "unreadable-literal-lint-fractions",
];

const LOOSENING_SWITCH_PREFIXES: [&str; 4] = ["accept-", "allow-", "avoid-", "suppress-"];

const TIGHTENING_SWITCH_PREFIXES: [&str; 5] =
    ["check-", "enforce-", "lint-", "missing-docs-", "warn-"];

const STRICT_SWITCHES: [&str; 1] = ["upper-case-acronyms-aggressive"];

/// Lists of exemptions: fewer entries report more.
const EXEMPTION_LISTS: [&str; 4] = [
    "allow-renamed-params-for",
    "doc-valid-idents",
    "ignore-interior-mutability",
    "allowed-",
];

/// Lists of bans: more entries report more.
const BAN_LISTS: [&str; 3] = [
    "await-holding-invalid-types",
    "disallowed-",
    "standard-macro-braces",
];

/// Settings with no stricter direction, only the preset's choice.
const EXACT: [&str; 4] = [
    "module-item-order-groupings",
    "module-items-ordered-within-groupings",
    "source-item-ordering",
    "trait-assoc-item-kinds-order",
];

/// Clippy's own default for every list: the entries it ships with.
const DEFAULT_LIST: &str = "..";

/// How one setting may change without loosening the preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Lower(i64),
    Higher(i64),
    /// The switch loosens when `true`; the field is its default.
    LooseWhenTrue(bool),
    StrictWhenTrue(bool),
    Subset,
    Superset,
    Exact,
    /// `msrv`: a lower one turns lints off, and Cargo's `rust-version` already sets it.
    NotHere,
    Unknown,
}

/// Which way a setting reports more.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    LowerIsStricter,
    HigherIsStricter,
}

/// One setting that's looser than the preset, by key.
#[derive(Debug)]
pub(super) struct Loosened {
    pub(super) key: String,
    /// Whether the file sets the key at all; an unset key is reported at the top.
    pub(super) set: bool,
    pub(super) message: String,
}

/// One key as the preset and the project set it. `None` on either side means that file leaves
/// the key at clippy's default.
struct Setting<'table> {
    key: &'table str,
    direction: Direction,
    preset: Option<&'table Value>,
    project: Option<&'table Value>,
}

impl Setting<'_> {
    /// Why the project's value is looser than the preset's, if it is.
    fn loosening(&self) -> Option<String> {
        let key = self.key;

        match self.direction {
            Direction::Lower(default) => self.threshold(default, Order::LowerIsStricter),
            Direction::Higher(default) => self.threshold(default, Order::HigherIsStricter),
            Direction::LooseWhenTrue(_) => self.switch(Order::HigherIsStricter),
            Direction::StrictWhenTrue(_) => self.switch(Order::LowerIsStricter),
            Direction::Subset | Direction::Superset => self.list(),
            Direction::Exact => {
                let wanted = self.preset?;

                (self.project.is_some_and(|have| have != wanted)).then(|| {
                    format!("`{key}` must stay {}, as the preset sets it", self.shown(Some(wanted)))
                })
            }
            Direction::NotHere => self.project.map(|_| format!("`{key}` in clippy.toml turns off every lint newer than it; set `rust-version` in Cargo.toml instead")),
            Direction::Unknown => {
                let changed = self.project.is_some_and(|have| Some(have) != self.preset);

                changed.then(|| format!("`cargo bs` doesn't know `{key}`, so it can't tell whether this loosens the preset; remove it, or report it upstream"))
            }
        }
    }

    /// A number where `order` says which way is stricter.
    fn threshold(&self, default: i64, order: Order) -> Option<String> {
        let wanted = number(self.key, self.preset, default)?;

        let Some(have) = number(self.key, self.project, default) else {
            return Some(self.unreadable());
        };

        let (looser, bound) = match order {
            Order::LowerIsStricter => (have > wanted, "at most"),
            Order::HigherIsStricter => (have < wanted, "at least"),
        };

        looser.then(|| {
            format!(
                "`{}` is {} here; the preset allows {bound} {}",
                self.key,
                self.shown(self.project),
                self.shown(self.preset)
            )
        })
    }

    /// A switch where `order` says whether `true` (higher) is stricter.
    fn switch(&self, order: Order) -> Option<String> {
        let default = matches!(
            self.direction,
            Direction::LooseWhenTrue(true) | Direction::StrictWhenTrue(true)
        );
        let wanted = self.preset.map_or(Some(default), Value::as_bool)?;

        let Some(have) = self.project.map_or(Some(default), Value::as_bool) else {
            return Some(self.unreadable());
        };

        let looser = match order {
            Order::HigherIsStricter => have && !wanted,
            Order::LowerIsStricter => wanted && !have,
        };

        looser.then(|| {
            format!(
                "`{}` is {} here; the preset has {}",
                self.key,
                self.shown(self.project),
                self.shown(self.preset)
            )
        })
    }

    fn list(&self) -> Option<String> {
        let wanted = list(self.preset)?;

        let Some(have) = list(self.project) else {
            return Some(self.unreadable());
        };

        if self.direction == Direction::Subset {
            let extra: Vec<&String> = have
                .iter()
                .filter(|entry| !wanted.contains(entry))
                .collect();

            (!extra.is_empty()).then(|| {
                format!(
                    "`{}` exempts {} on top of the preset's {}; it may only drop entries",
                    self.key,
                    quoted(&extra),
                    self.shown(self.preset)
                )
            })
        } else {
            let dropped: Vec<&String> = wanted
                .iter()
                .filter(|entry| !have.contains(entry))
                .collect();

            (!dropped.is_empty()).then(|| {
                format!(
                    "`{}` drops {} from the preset's bans; it may only add entries",
                    self.key,
                    quoted(&dropped)
                )
            })
        }
    }

    fn shown(&self, value: Option<&Value>) -> String {
        match (value, self.direction) {
            (Some(value), _) => format!("`{value}`"),
            (None, Direction::Lower(default) | Direction::Higher(default)) => {
                format!("clippy's default (`{default}`)")
            }
            (None, Direction::LooseWhenTrue(default) | Direction::StrictWhenTrue(default)) => {
                format!("clippy's default (`{default}`)")
            }
            (None, _) => "clippy's default".to_owned(),
        }
    }

    fn unreadable(&self) -> String {
        format!("`{}` isn't the kind of value clippy reads here", self.key)
    }
}

/// The settings in `project` that are looser than the same settings in `preset`.
pub(super) fn loosened(preset: &Table, project: &Table) -> Vec<Loosened> {
    let mut keys: Vec<&String> = preset.keys().chain(project.keys()).collect();
    keys.sort();
    keys.dedup();

    keys.into_iter()
        .filter_map(|key| {
            let setting = Setting {
                key,
                direction: direction(key),
                preset: preset.get(key),
                project: project.get(key),
            };
            let message = setting.loosening()?;

            Some(Loosened {
                key: key.clone(),
                set: project.contains_key(key),
                message,
            })
        })
        .collect()
}

fn direction(key: &str) -> Direction {
    let threshold = |table: &[(&str, i64)]| {
        table
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, default)| *default)
    };

    threshold(&LOWER_IS_STRICTER)
        .map(Direction::Lower)
        .or_else(|| threshold(&HIGHER_IS_STRICTER).map(Direction::Higher))
        .unwrap_or_else(|| direction_by_name(key))
}

/// Lists and switches, placed by their names.
fn direction_by_name(key: &str) -> Direction {
    let starts = |prefixes: &[&str]| prefixes.iter().any(|prefix| key.starts_with(prefix));
    let named = |names: &[&str]| names.contains(&key);

    if named(&EXACT) {
        Direction::Exact
    } else if key == "msrv" {
        Direction::NotHere
    } else if starts(&EXEMPTION_LISTS) || key.contains("-allowed-") {
        Direction::Subset
    } else if starts(&BAN_LISTS) {
        Direction::Superset
    } else if named(&LOOSE_BY_DEFAULT) {
        Direction::LooseWhenTrue(true)
    } else if starts(&LOOSENING_SWITCH_PREFIXES) {
        Direction::LooseWhenTrue(false)
    } else if named(&STRICT_BY_DEFAULT) {
        Direction::StrictWhenTrue(true)
    } else if starts(&TIGHTENING_SWITCH_PREFIXES) || named(&STRICT_SWITCHES) {
        Direction::StrictWhenTrue(false)
    } else {
        Direction::Unknown
    }
}

/// A threshold, with zero read as "off" (no limit) where it means that.
fn number(key: &str, value: Option<&Value>, default: i64) -> Option<i64> {
    let number = match value {
        Some(value) => value.as_integer()?,
        None => default,
    };

    Some(if number == 0 && ZERO_IS_OFF.contains(&key) {
        i64::MAX
    } else {
        number
    })
}

/// A list's entries by identity: a string, or a table's `path` or `name`.
fn list(value: Option<&Value>) -> Option<Vec<String>> {
    let Some(value) = value else {
        return Some(vec![DEFAULT_LIST.to_owned()]);
    };

    value
        .as_array()?
        .iter()
        .map(|entry| match entry {
            Value::String(text) => Some(text.clone()),
            Value::Table(table) => table
                .get("path")
                .or_else(|| table.get("name"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        })
        .collect()
}

fn quoted(entries: &[&String]) -> String {
    entries
        .iter()
        .map(|entry| format!("`{entry}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use toml::Table;

    use super::loosened;

    fn loosened_keys(preset: &str, project: &str) -> Vec<String> {
        loosened(
            &preset.parse::<Table>().unwrap(),
            &project.parse::<Table>().unwrap(),
        )
        .into_iter()
        .map(|found| found.key)
        .collect()
    }

    #[test]
    fn a_raised_threshold_is_looser() {
        let found = loosened_keys(
            "excessive-nesting-threshold = 5\n",
            "excessive-nesting-threshold = 9\ntoo-many-lines-threshold = 100000\ncognitive-complexity-threshold = 1000\n",
        );

        assert_eq!(
            found,
            [
                "cognitive-complexity-threshold",
                "excessive-nesting-threshold",
                "too-many-lines-threshold"
            ]
        );
    }

    #[test]
    fn a_tightened_threshold_passes() {
        let found = loosened_keys(
            "too-many-arguments-threshold = 5\nmin-ident-chars-threshold = 1\n",
            "too-many-arguments-threshold = 3\nmin-ident-chars-threshold = 2\ntoo-many-lines-threshold = 60\n",
        );

        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn a_missing_threshold_reads_as_clippys_default() {
        let found = loosened_keys(
            "excessive-nesting-threshold = 5\ntoo-many-arguments-threshold = 5\n",
            "",
        );

        assert_eq!(
            found,
            [
                "excessive-nesting-threshold",
                "too-many-arguments-threshold"
            ]
        );
    }

    #[test]
    fn switches_may_only_turn_stricter() {
        let preset = "allow-unwrap-in-tests = true\n";
        let found = loosened_keys(
            preset,
            "allow-unwrap-in-tests = false\nallow-print-in-tests = true\navoid-breaking-exported-api = false\n",
        );

        assert_eq!(found, ["allow-print-in-tests"]);
    }

    #[test]
    fn exemptions_may_shrink_and_bans_may_grow() {
        let preset =
            "allowed-idents-below-min-chars = [\"i\", \"j\"]\ndisallowed-names = [\"foo\"]\n";

        assert_eq!(
            loosened_keys(
                preset,
                "allowed-idents-below-min-chars = [\"i\"]\ndisallowed-names = [\"foo\", \"bar\"]\n"
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            loosened_keys(
                preset,
                "allowed-idents-below-min-chars = [\"i\", \"j\", \"x\"]\ndisallowed-names = []\n"
            ),
            ["allowed-idents-below-min-chars", "disallowed-names"]
        );
    }

    #[test]
    fn unknown_keys_and_msrv_are_reported() {
        let found = loosened_keys("", "msrv = \"1.0\"\nsome-future-knob = 3\n");

        assert_eq!(found, ["msrv", "some-future-knob"]);
    }
}
