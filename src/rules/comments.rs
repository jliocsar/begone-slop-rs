//! `no-commented-out-code`: code that's commented out is deleted code that wasn't deleted.
//!
//! Version control keeps old code; a comment keeps it in everyone's way. A run of `//` lines
//! counts as code when it parses as Rust statements and has code punctuation in it (prose
//! rarely does both). Doc comments, `SAFETY:` notes and tool directives are never code.

use super::{Finding, Rule};
use crate::source::Source;

const CODE_MARKS: [&str; 7] = [";", "{", "}", "=>", "::", "->", "()"];

const DIRECTIVES: [&str; 5] = [
    "SAFETY:",
    "bs-allow(",
    "ast-grep-ignore",
    "TODO",
    "rustfmt::",
];

/// One run of plain `//` comment lines: where it starts and its text without the markers.
struct CommentRun {
    line: usize,
    text: String,
}

pub(super) fn check(source: &Source) -> Vec<Finding> {
    runs(source)
        .into_iter()
        .filter(|run| looks_like_code(&run.text))
        .map(|run| Finding {
            line: run.line,
            rule: Rule::NoCommentedOutCode,
            message: "this comment is code; delete it (git keeps the history)".to_owned(),
            edit: None,
        })
        .collect()
}

fn runs(source: &Source) -> Vec<CommentRun> {
    let plain_comment = |line: &str| {
        let trimmed = line.trim_start();
        let is_doc = trimmed.starts_with("///") || trimmed.starts_with("//!");

        (trimmed.starts_with("//") && !is_doc)
            .then(|| trimmed.trim_start_matches('/').trim().to_owned())
    };

    source
        .lines
        .iter()
        .zip(1..)
        .fold(Vec::new(), |mut runs: Vec<CommentRun>, (line, number)| {
            let previous_is_comment =
                number > 1 && source.line(number - 1).and_then(plain_comment).is_some();

            match (plain_comment(line), runs.last_mut()) {
                (Some(text), Some(run)) if previous_is_comment => {
                    run.text.push('\n');
                    run.text.push_str(&text);
                }
                (Some(text), _) => runs.push(CommentRun { line: number, text }),
                (None, _) => {}
            }

            runs
        })
}

fn looks_like_code(text: &str) -> bool {
    let has_marks = CODE_MARKS.iter().any(|mark| text.contains(mark));
    let is_directive = DIRECTIVES.iter().any(|directive| text.contains(directive));

    has_marks && !is_directive && syn::parse_str::<syn::Block>(&format!("{{\n{text}\n}}")).is_ok()
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn commented_out_statements_are_reported() {
        let text =
            "fn run() {\n    // let old = legacy(input);\n    // old.flush();\n    go();\n}\n";

        assert_eq!(lines_of(Rule::NoCommentedOutCode, text), [2]);
    }

    #[test]
    fn prose_is_left_alone() {
        let text = "fn run() {\n    // Flush before closing; the socket drops what's buffered.\n    // See `Socket::close` for why.\n    go();\n}\n";

        assert_eq!(lines_of(Rule::NoCommentedOutCode, text), NONE);
    }

    #[test]
    fn a_safety_note_is_left_alone() {
        let text =
            "fn run() {\n    // SAFETY: the pointer came from Box::into_raw();\n    go();\n}\n";

        assert_eq!(lines_of(Rule::NoCommentedOutCode, text), NONE);
    }
}
