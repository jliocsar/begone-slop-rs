//! The syntax rules hold for this repo's own code. The gate's `cargo bs` covers the pattern
//! rules too; this test keeps `cargo test` alone from passing slop.

use std::path::{Path, PathBuf};

use cargo_bs::check::{self, Mode};
use cargo_bs::config::Config;
use cargo_bs::files;

#[test]
fn this_repo_passes_its_own_rules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let roots: Vec<PathBuf> = ["src", "tests"]
        .iter()
        .map(|name| root.join(name))
        .collect();
    let config = Config::load(root).unwrap();
    let problems: Vec<String> = files::rust_files(&roots)
        .unwrap()
        .iter()
        .flat_map(|path| {
            let report = check::check_file(path, Mode::Report, &config).unwrap();

            report.findings.into_iter().map(move |finding| {
                format!("{}:{}: {}", path.display(), finding.line, finding.message)
            })
        })
        .collect();

    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
