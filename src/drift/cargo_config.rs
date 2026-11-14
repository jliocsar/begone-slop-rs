//! `cargo-config-drift`: Cargo's own config can switch lints off without touching a manifest.
//!
//! `rustflags` with `-A` or `--cap-lints` beat the `[lints]` table (they come later on rustc's
//! command line), `[env] CLIPPY_CONF_DIR` swaps in another clippy.toml, a rustc wrapper can
//! rewrite every invocation, and an alias named `bs` or `clippy` replaces the command itself.
//! The same flags in the environment of `cargo bs` mean this run isn't the real gate either.

use toml::{Table, Value};

use crate::lints::{self, Entry, Level};

/// Variables that change what clippy or rustc check, whatever their value.
const GATE_VARIABLES: [&str; 6] = [
    "CLIPPY_ARGS",
    "CLIPPY_CONF_DIR",
    "RUSTC",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTC_WRAPPER",
    "RUSTFLAGS",
];

/// `[build]` keys that replace or wrap rustc.
const COMPILER_KEYS: [&str; 3] = ["rustc", "rustc-wrapper", "rustc-workspace-wrapper"];

/// Aliases that would replace a step of the gate.
const GATE_ALIASES: [&str; 2] = ["clippy", "fmt"];

/// How this repo runs the tool it builds, minus the flags in [`QUIET_RUN_FLAGS`].
const BOOTSTRAP_FORMS: [&[&str]; 2] = [
    &["run", "--bin", "cargo-bs", "--", "bs"],
    &["run", "--bin=cargo-bs", "--", "bs"],
];

/// Flags `cargo run` may take in the `bs` alias without changing what runs.
const QUIET_RUN_FLAGS: [&str; 5] = ["--frozen", "--locked", "--offline", "--quiet", "-q"];

/// A rustc option that sets lint levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LintOption {
    Allow,
    CapLints,
    Warn,
}

/// An environment variable that changes what the gate checks.
#[derive(Debug, PartialEq, Eq)]
enum Variable {
    ClippyConfig,
    Flags(Vec<String>),
}

/// One problem, by the config key it's under (used to find its line).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Bypass {
    pub(super) key: String,
    pub(super) message: String,
}

impl LintOption {
    /// `-A x`, `-Ax`, `--allow x`, `--allow=x` and the same for `--cap-lints`, `-W`, `--warn`
    /// and `--force-warn`, with the value when it's attached to the flag.
    fn parse(flag: &str) -> Option<(Self, Option<String>)> {
        let (option, attached) = match flag.split_once('=') {
            Some((option, value)) if option.starts_with("--") => (option, Some(value.to_owned())),
            _ if flag.len() > 2 && (flag.starts_with("-A") || flag.starts_with("-W")) => {
                (flag.get(..2)?, flag.get(2..).map(str::to_owned))
            }
            _ => (flag, None),
        };

        let parsed = match option {
            "-A" | "--allow" => Self::Allow,
            "--cap-lints" => Self::CapLints,
            "-W" | "--warn" | "--force-warn" => Self::Warn,
            _ => return None,
        };

        Some((parsed, attached))
    }
}

impl Variable {
    fn parse(name: &str, value: &str) -> Option<Self> {
        let spaced = || value.split_whitespace().map(str::to_owned).collect();

        match name {
            "CLIPPY_CONF_DIR" => Some(Self::ClippyConfig),
            "CARGO_ENCODED_RUSTFLAGS" => Some(Self::Flags(
                value.split('\u{1f}').map(str::to_owned).collect(),
            )),
            "RUSTFLAGS" => Some(Self::Flags(spaced())),
            _ if name.starts_with("CARGO_") && name.ends_with("_RUSTFLAGS") => {
                Some(Self::Flags(spaced()))
            }
            _ => None,
        }
    }
}

/// What in a `.cargo/config.toml` loosens the gate. `preset` is the preset's lints table,
/// which decides whether a `-W` lowers anything.
pub(super) fn config_bypasses(config: &Table, preset: &[Entry]) -> Vec<Bypass> {
    let mut found = Vec::new();
    let build = config.get("build").and_then(Value::as_table);
    let host = config.get("host");
    let flag_lists = build
        .and_then(|build| build.get("rustflags"))
        .into_iter()
        .chain(host.and_then(|host| host.get("rustflags")))
        .chain(per_target(host))
        .chain(per_target(config.get("target")));

    for flags in flag_lists {
        found.extend(flag_bypasses("rustflags", &words(flags), preset));
    }

    for key in COMPILER_KEYS {
        if build.is_some_and(|build| build.contains_key(key)) {
            found.push(Bypass {
                key: key.to_owned(),
                message: format!("`build.{key}` wraps or replaces rustc, so it can drop any lint the gate turns on"),
            });
        }
    }

    for (variable, _) in config
        .get("env")
        .and_then(Value::as_table)
        .into_iter()
        .flatten()
    {
        if is_gate_variable(variable) {
            found.push(Bypass {
                key: variable.clone(),
                message: format!(
                    "`[env] {variable}` changes what rustc or clippy check behind the gate's back"
                ),
            });
        }
    }

    for (name, command) in config
        .get("alias")
        .and_then(Value::as_table)
        .into_iter()
        .flatten()
    {
        found.extend(alias_bypass(name, command));
    }

    found
}

/// What in `cargo bs`'s own environment means this run isn't the gate.
pub(super) fn environment_bypasses(
    variables: &[(String, String)],
    preset: &[Entry],
) -> Vec<Bypass> {
    variables
        .iter()
        .flat_map(|(name, value)| match Variable::parse(name, value) {
            Some(Variable::ClippyConfig) => vec![Bypass {
                key: name.clone(),
                message: format!("`{name}` points clippy at another clippy.toml; unset it"),
            }],
            Some(Variable::Flags(flags)) => flag_bypasses(name, &flags, preset),
            None => Vec::new(),
        })
        .collect()
}

/// `rustflags` under each `[target.<triple>]` (or `[host.<triple>]`) table.
fn per_target(section: Option<&Value>) -> impl Iterator<Item = &Value> {
    section
        .and_then(Value::as_table)
        .into_iter()
        .flatten()
        .filter_map(|(_, table)| table.get("rustflags"))
}

fn is_gate_variable(variable: &str) -> bool {
    GATE_VARIABLES.contains(&variable)
        || variable.ends_with("RUSTFLAGS")
        || variable.starts_with("CARGO_BUILD_RUSTC")
}

/// `rustflags` as a string of words or an array of them.
fn words(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => text.split_whitespace().map(str::to_owned).collect(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// The lint-lowering flags in a rustc command line: any allow or lint cap, and a warn on a
/// lint the preset denies.
fn flag_bypasses(key: &str, flags: &[String], preset: &[Entry]) -> Vec<Bypass> {
    let mut found = Vec::new();
    let mut rest = flags.iter();

    while let Some(flag) = rest.next() {
        let Some((option, attached)) = LintOption::parse(flag) else {
            continue;
        };

        let value = attached
            .or_else(|| rest.next().cloned())
            .unwrap_or_default();

        let lowers = match option {
            LintOption::Allow | LintOption::CapLints => true,
            LintOption::Warn => lowers_a_denied_lint(&value, preset),
        };

        if lowers {
            found.push(Bypass {
                key: key.to_owned(),
                message: format!(
                    "`{flag}{}` in {key} turns lints down behind the lints table's back; delete it",
                    if flag.ends_with(&value) {
                        String::new()
                    } else {
                        format!(" {value}")
                    }
                ),
            });
        }
    }

    found
}

/// Whether warning on `name` (a lint or a group) lowers a lint the preset denies or forbids.
fn lowers_a_denied_lint(name: &str, preset: &[Entry]) -> bool {
    let name = lints::current_name(&name.replace('-', "_")).to_owned();

    let covered: Vec<&str> = if lints::members(&name).is_empty() {
        vec![name.as_str()]
    } else {
        lints::members(&name).to_vec()
    };

    covered.iter().any(|lint| {
        lints::resolve(preset, lint)
            .level
            .is_some_and(|level| level > Level::Warn)
    })
}

fn alias_bypass(name: &str, command: &Value) -> Option<Bypass> {
    let message = if GATE_ALIASES.contains(&name) {
        format!("the alias `{name}` replaces `cargo {name}`, a step of the gate; delete it")
    } else if name == "bs" && !runs_in_tree_binary(&words(command)) {
        "the alias `bs` replaces `cargo bs`; only `run --bin cargo-bs -- bs` (this repo's bootstrap) may stand in for it".to_owned()
    } else {
        return None;
    };

    Some(Bypass {
        key: name.to_owned(),
        message,
    })
}

/// `run [--quiet] --bin cargo-bs -- bs`, and nothing after `bs`.
fn runs_in_tree_binary(words: &[String]) -> bool {
    let separator = words
        .iter()
        .position(|word| word == "--")
        .unwrap_or(words.len());
    let (options, after) = words.split_at(separator);
    let kept: Vec<&str> = options
        .iter()
        .map(String::as_str)
        .filter(|word| !QUIET_RUN_FLAGS.contains(word))
        .chain(after.iter().map(String::as_str))
        .collect();

    BOOTSTRAP_FORMS.iter().any(|form| kept == *form)
}

#[cfg(test)]
mod tests {
    use toml::Table;

    use super::{config_bypasses, environment_bypasses};
    use crate::preset::lint_entries;

    fn bypassed_keys(config: &str) -> Vec<String> {
        config_bypasses(&config.parse::<Table>().unwrap(), &lint_entries().unwrap())
            .into_iter()
            .map(|bypass| bypass.key)
            .collect()
    }

    #[test]
    fn this_repos_bootstrap_alias_passes() {
        let config = "[alias]\nbs = \"run --quiet --bin cargo-bs -- bs\"\n\n[build]\njobs = 4\nrustflags = [\"-D\", \"warnings\", \"-W\", \"clippy::pedantic\"]\n";

        assert_eq!(bypassed_keys(config), Vec::<String>::new());
    }

    #[test]
    fn allow_flags_and_lint_caps_are_reported() {
        let config = "[build]\nrustflags = [\"-A\", \"clippy::unwrap_used\", \"-Aclippy::pedantic\"]\n\n[target.x86_64-unknown-linux-gnu]\nrustflags = \"--cap-lints=warn\"\n";

        assert_eq!(
            bypassed_keys(config),
            ["rustflags", "rustflags", "rustflags"]
        );
    }

    #[test]
    fn warning_on_a_denied_lint_is_reported() {
        let config = "[build]\nrustflags = [\"-W\", \"unsafe_code\"]\n";

        assert_eq!(bypassed_keys(config), ["rustflags"]);
    }

    #[test]
    fn env_wrappers_and_aliases_are_reported() {
        let config = "[env]\nCLIPPY_CONF_DIR = \"/nowhere\"\nPATH_EXTRA = \"x\"\n\n[build]\nrustc-wrapper = \"quiet-rustc\"\n\n[alias]\nbs = \"check\"\nclippy = \"check\"\nbuild-all = \"build --workspace\"\n";

        assert_eq!(
            bypassed_keys(config),
            ["rustc-wrapper", "CLIPPY_CONF_DIR", "bs", "clippy"]
        );
    }

    #[test]
    fn a_bs_alias_with_extra_arguments_is_reported() {
        let config = "[alias]\nbs = \"run --bin cargo-bs -- bs --no-ast-grep\"\n";

        assert_eq!(bypassed_keys(config), ["bs"]);
    }

    #[test]
    fn the_environment_is_checked_too() {
        let variables = [
            (
                "RUSTFLAGS".to_owned(),
                "-C debuginfo=0 -A dead_code".to_owned(),
            ),
            (
                "CARGO_ENCODED_RUSTFLAGS".to_owned(),
                "--cap-lints\u{1f}allow".to_owned(),
            ),
            ("CLIPPY_CONF_DIR".to_owned(), "/tmp".to_owned()),
            ("CARGO_TARGET_DIR".to_owned(), "/tmp/target".to_owned()),
        ];
        let keys: Vec<String> = environment_bypasses(&variables, &lint_entries().unwrap())
            .into_iter()
            .map(|bypass| bypass.key)
            .collect();

        assert_eq!(
            keys,
            ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CLIPPY_CONF_DIR"]
        );
    }
}
