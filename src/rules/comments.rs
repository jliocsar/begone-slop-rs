//! `no-commented-out-code`: code that's commented out is deleted code that wasn't deleted.
//!
//! Version control keeps old code; a comment keeps it in everyone's way. A run of `//` lines
//! counts as code when it parses as Rust statements and has code punctuation in it (prose
//! rarely does both). Doc comments, `SAFETY:` notes and tool directives are never code.

use super::{FileContext, Finding, Rule};
use crate::source::{CommentKind, Source};

const CODE_MARKS: [&str; 7] = [";", "{", "}", "=>", "::", "->", "()"];

const DIRECTIVES: [&str; 5] = [
    "SAFETY:",
    "bs-allow(",
    "ast-grep-ignore",
    "TODO",
    "rustfmt::",
];

/// One run of plain `//` comment lines: where it starts and ends, and its text without the
/// markers.
struct CommentRun {
    line: usize,
    last: usize,
    text: String,
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    runs(file.source)
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

/// Plain `//` comments that each sit on their own line, grouped into runs of neighbours. The
/// lexer's comments, so a line inside a string that starts with `//` is never one.
fn runs(source: &Source) -> Vec<CommentRun> {
    source
        .comments()
        .iter()
        .filter(|comment| comment.kind == CommentKind::Line && !comment.after_code)
        .fold(Vec::new(), |mut runs: Vec<CommentRun>, comment| {
            let text = comment.text.trim_start_matches('/').trim();

            match runs.last_mut() {
                Some(run) if run.last + 1 == comment.line => {
                    run.text.push('\n');
                    run.text.push_str(text);
                    run.last = comment.line;
                }
                _ => runs.push(CommentRun {
                    line: comment.line,
                    last: comment.line,
                    text: text.to_owned(),
                }),
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

    #[test]
    fn code_inside_a_raw_string_isnt_a_comment() {
        let text =
            "const FIXTURE: &str = r#\"\n// let old = legacy(input);\n// old.flush();\n\"#;\n";

        assert_eq!(lines_of(Rule::NoCommentedOutCode, text), NONE);
    }

    #[test]
    fn code_after_code_on_its_line_isnt_a_run() {
        let text = "fn run() {\n    go(); // let old = legacy(input);\n}\n";

        assert_eq!(lines_of(Rule::NoCommentedOutCode, text), NONE);
    }
}
