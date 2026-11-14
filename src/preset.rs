//! The presets `cargo bs` hands out and holds projects to. They're this repo's own config
//! files, so the tool runs under exactly what it gives everyone else.

use toml::{Table, Value};

use crate::lints::{self, Entry};

/// The `[workspace.lints]` tables, as `preset/lints.toml` holds them.
pub const LINTS: &str = include_str!("../preset/lints.toml");

/// The preset `clippy.toml`, which is this repo's own.
pub const CLIPPY: &str = include_str!("../clippy.toml");

/// The preset `rustfmt.toml`, which is this repo's own.
pub const RUSTFMT: &str = include_str!("../rustfmt.toml");

/// The preset's lints table as entries.
///
/// # Errors
///
/// Fails when the embedded table doesn't parse, which a test rules out.
pub fn lint_entries() -> Result<Vec<Entry>, toml::de::Error> {
    let table = LINTS.parse::<Table>()?;
    let lints = table
        .get("workspace")
        .and_then(|workspace| workspace.get("lints"))
        .and_then(Value::as_table);

    Ok(lints
        .map(|lints| lints::entries(lints).0)
        .unwrap_or_default())
}

/// The preset `clippy.toml` as a table.
///
/// # Errors
///
/// Fails when the embedded file doesn't parse, which a test rules out.
pub fn clippy_settings() -> Result<Table, toml::de::Error> {
    CLIPPY.parse::<Table>()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{LINTS, clippy_settings, lint_entries};

    #[test]
    fn this_repo_uses_the_lints_it_hands_out() {
        let manifest =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();

        assert!(
            manifest.contains(LINTS),
            "Cargo.toml's lints drifted from preset/lints.toml"
        );
    }

    #[test]
    fn the_embedded_presets_parse() {
        let entries = lint_entries().unwrap();

        assert!(entries.iter().any(|entry| entry.name == "clippy::pedantic"));
        assert!(
            clippy_settings()
                .unwrap()
                .contains_key("excessive-nesting-threshold")
        );
    }
}
