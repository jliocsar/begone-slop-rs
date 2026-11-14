//! `cargo bs init`: puts the presets into a project.
//!
//! The presets are this repo's own config files, so the tool is held to exactly what it hands
//! out. Existing files are never overwritten, and the lints table is printed, not merged:
//! Cargo.toml is the project's, and a silent edit to it is a surprise.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

/// The `[workspace.lints]` tables, as `preset/lints.toml` holds them.
pub const LINTS: &str = include_str!("../preset/lints.toml");

const PRESET_FILES: [(&str, &str); 2] = [
    ("clippy.toml", include_str!("../clippy.toml")),
    ("rustfmt.toml", include_str!("../rustfmt.toml")),
];

/// What `init` did with one preset file.
pub enum Outcome {
    Written(&'static str),
    Kept(&'static str),
}

/// Writes each preset file into `root` unless the project already has one.
///
/// # Errors
///
/// Fails when a file can't be created or written.
pub fn write_presets(root: &Path) -> io::Result<Vec<Outcome>> {
    PRESET_FILES
        .iter()
        .map(|(name, text)| {
            let created = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join(name));

            match created {
                Ok(mut file) => file
                    .write_all(text.as_bytes())
                    .map(|()| Outcome::Written(name)),
                Err(cause) if cause.kind() == io::ErrorKind::AlreadyExists => {
                    Ok(Outcome::Kept(name))
                }
                Err(cause) => Err(cause),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::LINTS;

    #[test]
    fn this_repo_uses_the_lints_it_hands_out() {
        let manifest =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();

        assert!(
            manifest.contains(LINTS),
            "Cargo.toml's lints drifted from preset/lints.toml"
        );
    }
}
