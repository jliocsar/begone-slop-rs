//! `waiver-needs-reason`: every waiver names what it waives and says why, where it can't leak.
//!
//! A `// bs-allow(<rule>): <reason>` waives one syntax rule on its line or the next, and the
//! reason is a sentence. The gate's own rules can't be waived, so naming one is a finding, and
//! so is a name no rule has. Only a plain `//` comment waives: a doc or block comment that
//! looks like one is a finding, since it reads as a waiver and does nothing.
//!
//! ast-grep honours any comment that opens with `ast-grep-ignore`, doc and block comments
//! included. A bare one waives every pattern rule, one on the first line followed by a blank
//! line waives the whole file, and one after code can waive lines above it. So the only form
//! that passes is a plain `//` comment on its own line, naming pattern rules and a reason
//! (`// ast-grep-ignore: no-catch-unwind -- the FFI boundary must not unwind`), right above
//! the code it waives. The check runs here, not as a pattern rule, so ast-grep can't waive it.

use super::{FileContext, Finding, Rule};
use crate::ast_grep::RULES;
use crate::source::{Comment, CommentKind, MIN_REASON_WORDS, Source, is_real_reason};

const BS_ALLOW: &str = "bs-allow";
const AST_GREP_IGNORE: &str = "ast-grep-ignore";

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    file.source
        .comments
        .iter()
        .filter_map(|comment| {
            let body = directive(comment);
            let problem = if body.starts_with(BS_ALLOW) {
                bs_allow_problem(comment, body)
            } else if body.starts_with(AST_GREP_IGNORE) {
                ignore_problem(file.source, comment, body)
            } else {
                None
            }?;

            Some(Finding {
                line: comment.line,
                rule: Rule::WaiverNeedsReason,
                message: problem,
                edit: None,
            })
        })
        .collect()
}

/// The comment's text without its markers, which is what ast-grep reads.
fn directive(comment: &Comment) -> &str {
    comment
        .text
        .trim_start_matches(['*', '!', '/'])
        .trim_start()
}

fn bs_allow_problem(comment: &Comment, body: &str) -> Option<String> {
    let parsed = body
        .strip_prefix("bs-allow(")
        .and_then(|rest| rest.split_once("):"));

    if comment.kind != CommentKind::Line {
        Some("only a plain `//` comment waives a rule; this one does nothing".to_owned())
    } else if let Some((name, reason)) = parsed {
        rule_problem(name).or_else(|| reason_problem(reason))
    } else {
        Some("write a waiver as `// bs-allow(<rule>): <reason>`".to_owned())
    }
}

fn rule_problem(name: &str) -> Option<String> {
    match Rule::from_name(name) {
        Some(rule) if rule.is_waivable() => None,
        Some(_) => Some(format!(
            "`{name}` guards the gate itself, so it can't be waived; fix the code"
        )),
        None if pattern_rule(name) => Some(format!(
            "`{name}` is a pattern rule; waive it with `// ast-grep-ignore: {name} -- <reason>`"
        )),
        None => Some(format!("no rule is called `{name}`")),
    }
}

fn reason_problem(reason: &str) -> Option<String> {
    (!is_real_reason(reason)).then(|| {
        format!(
            "the reason has to be a sentence ({MIN_REASON_WORDS} words or more) that says why the rule doesn't fit here"
        )
    })
}

fn ignore_problem(source: &Source, comment: &Comment, body: &str) -> Option<String> {
    let parsed = body
        .strip_prefix("ast-grep-ignore:")
        .and_then(|rest| rest.split_once(" -- "));
    let next_is_code = source.line(comment.line + 1).is_some_and(is_code_line);

    if comment.kind != CommentKind::Line {
        Some("an `ast-grep-ignore` goes in a plain `//` comment; ast-grep honours this one too, so it waives every pattern rule".to_owned())
    } else if comment.after_code {
        Some("put the `ast-grep-ignore` on its own line, above the code it waives".to_owned())
    } else if let Some((names, reason)) = parsed {
        names
            .split(',')
            .map(str::trim)
            .find(|name| !pattern_rule(name))
            .map(|name| format!("`{name}` isn't a pattern rule, so this waiver names nothing"))
            .or_else(|| reason_problem(reason))
            .or_else(|| {
                (!next_is_code).then(|| {
                    "the next line has to be the code it waives; above a blank line it can waive the whole file".to_owned()
                })
            })
    } else {
        Some("an `ast-grep-ignore` names its rules and says why: `// ast-grep-ignore: <rule> -- <reason>`".to_owned())
    }
}

fn pattern_rule(name: &str) -> bool {
    RULES.iter().any(|(id, _)| *id == name)
}

fn is_code_line(line: &str) -> bool {
    let trimmed = line.trim_start();

    !trimmed.is_empty() && !trimmed.starts_with("//") && !trimmed.starts_with("/*")
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    fn waiver_lines(text: &str) -> Vec<usize> {
        lines_of(Rule::WaiverNeedsReason, text)
    }

    #[test]
    fn a_waiver_with_a_sentence_passes() {
        let text = "// bs-allow(complexity): one arm per key name reads better\nfn key() {}\n";

        assert_eq!(waiver_lines(text), NONE);
    }

    #[test]
    fn a_short_reason_is_reported() {
        let text = "// bs-allow(complexity): fine\nfn key() {}\n";

        assert_eq!(waiver_lines(text), [1]);
    }

    #[test]
    fn a_gate_rule_cant_be_waived() {
        let text = "// bs-allow(no-forbidden-expects): the lint is noisy in this file\n#[expect(clippy::unwrap_used, reason = \"x\")]\nfn key() {}\n";

        assert_eq!(waiver_lines(text), [1]);
        assert_eq!(lines_of(Rule::NoForbiddenExpects, text), [2]);
    }

    #[test]
    fn a_waiver_of_the_waiver_rule_does_nothing() {
        let text = "// bs-allow(waiver-needs-reason): we really do not care here\n// bs-allow(complexity): ok\nfn key() {}\n";

        assert_eq!(waiver_lines(text), [1, 2]);
    }

    #[test]
    fn an_unknown_rule_is_reported() {
        let text = "// bs-allow(no-such-rule): there is a reason for this one\nfn key() {}\n";

        assert_eq!(waiver_lines(text), [1]);
    }

    #[test]
    fn a_doc_comment_waiver_is_reported() {
        let text = "/// bs-allow(complexity): one arm per key name reads better\nfn key() {}\n";

        assert_eq!(waiver_lines(text), [1]);
    }

    #[test]
    fn a_named_ignore_with_a_reason_passes() {
        let text = "fn run() {\n    // ast-grep-ignore: no-catch-unwind, no-untyped-json -- the FFI boundary must not unwind\n    guard(work);\n}\n";

        assert_eq!(waiver_lines(text), NONE);
    }

    #[test]
    fn every_ignore_hole_is_reported() {
        let text = "// ast-grep-ignore: no-catch-unwind -- the whole file is generated\n\nfn run() {\n    // ast-grep-ignore\n    guard(work);\n    /* ast-grep-ignore: no-catch-unwind -- a block comment waiver */\n    guard(work);\n    guard(work); // ast-grep-ignore: no-catch-unwind -- trailing on its line\n    // ast-grep-ignore: waiver-needs-reason -- turn the check off here\n    guard(work);\n    // ast-grep-ignore: no-catch-unwind\n    guard(work);\n    /// ast-grep-ignore: no-catch-unwind -- a doc comment waiver\n    guard(work);\n}\n";

        assert_eq!(waiver_lines(text), [1, 4, 6, 8, 9, 11, 13]);
    }

    #[test]
    fn prose_that_mentions_a_waiver_passes() {
        let text = "// see the ast-grep-ignore docs, and bs-allow(rule) for syntax rules\nconst MARK: &str = \"// ast-grep-ignore\";\n";

        assert_eq!(waiver_lines(text), NONE);
    }
}
