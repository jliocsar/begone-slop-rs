//! The syntax rules hold for this repo's own code. The gate's `cargo bs` covers the pattern
//! rules too; this test keeps `cargo test` alone from passing slop.

use std::path::{Path, PathBuf};

use cargo_bs::{check, files};

#[test]
fn this_repo_passes_its_own_rules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let roots: Vec<PathBuf> = ["src", "tests"]
        .iter()
        .map(|name| root.join(name))
        .collect();
    let mut problems = Vec::new();

    for path in files::rust_files(&roots).unwrap() {
        let report = check::check_file(&path, false).unwrap();
        problems.extend(
            report
                .findings
                .iter()
                .map(|finding| format!("{}:{}: {}", path.display(), finding.line, finding.message)),
        );
    }

    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
