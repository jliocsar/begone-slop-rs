//! `waiver-needs-reason`, `unknown-waiver` and `unused-waiver`: every waiver names a rule,
//! says why, waives something, and can't leak.
//!
//! A `// bs-allow(<rule>): <reason>` waives one syntax rule on its line or the next, and the
//! reason is a sentence. The gate's own rules can't be waived, so naming one is a finding, and
//! so is a name no rule has (a typo waives nothing). A waiver that takes no finding off the
//! report is a finding too: it's left over from code that changed, and it would hide the next
//! finding there. Only a plain `//` comment waives: a doc or block comment that looks like one
//! is a finding, since it reads as a waiver and does nothing.
//!
//! ast-grep honours any comment that opens with `ast-grep-ignore`, doc and block comments
//! included. A bare one waives every pattern rule, one on the first line followed by a blank
//! line waives the whole file, and one after code can waive lines above it. So the only form
//! that passes is a plain `//` comment on its own line, naming pattern rules and a reason
//! (`// ast-grep-ignore: no-catch-unwind -- the FFI boundary must not unwind`), right above
//! the code it waives. The check runs here, not as a pattern rule, so ast-grep can't waive it.

use super::{FileContext, Finding, Rule, UnknownRule};
use crate::ast_grep::RULES;
use crate::source::{Comment, CommentKind, MIN_REASON_WORDS, Source, is_real_reason};

const BS_ALLOW: &str = "bs-allow";
const AST_GREP_IGNORE: &str = "ast-grep-ignore";

/// One `// bs-allow(<rule>): <reason>` in a plain `//` comment, read once per file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Waiver {
    pub(super) line: usize,
    pub(super) rule: Result<Rule, UnknownRule>,
    pub(super) reason: String,
}

impl Waiver {
    /// Whether the waiver takes `finding` off the report: it's in force, names the finding's
    /// rule, and sits on the finding's line or the one above.
    pub(super) fn waives(&self, finding: &Finding) -> bool {
        self.is_in_force()
            && self.rule == Ok(finding.rule)
            && (self.line == finding.line || self.line + 1 == finding.line)
    }

    /// A waiver of a rule that can be waived, with a sentence for a reason. Any other waiver
    /// waives nothing, and [`check`] says why.
    fn is_in_force(&self) -> bool {
        matches!(self.rule, Ok(rule) if rule.is_waivable()) && is_real_reason(&self.reason)
    }
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    file.source
        .comments()
        .iter()
        .filter_map(|comment| {
            let body = directive(comment);
            let (rule, message) = if body.starts_with(BS_ALLOW) {
                bs_allow_problem(comment, body)
            } else if body.starts_with(AST_GREP_IGNORE) {
                ignore_problem(file.source, comment, body)
                    .map(|message| (Rule::WaiverNeedsReason, message))
            } else {
                None
            }?;

            Some(Finding {
                line: comment.line,
                rule,
                message,
                edit: None,
            })
        })
        .collect()
}

/// Every well-formed `bs-allow` in a plain `//` comment. A malformed one, or one in a doc or
/// block comment, waives nothing, and [`check`] reports it.
pub(super) fn parse(source: &Source) -> Vec<Waiver> {
    source
        .comments()
        .iter()
        .filter(|comment| comment.kind == CommentKind::Line)
        .filter_map(|comment| {
            let (name, reason) = split_bs_allow(directive(comment))?;

            Some(Waiver {
                line: comment.line,
                rule: name.parse(),
                reason: reason.to_owned(),
            })
        })
        .collect()
}

/// The waivers in force that took no finding off the report: a waiver whose finding was
/// fixed long ago, or that sits a line too far from it, only hides the next one.
pub(super) fn unused(waivers: &[Waiver], waived: &[Finding]) -> Vec<Finding> {
    waivers
        .iter()
        .filter(|waiver| waiver.is_in_force())
        .filter(|waiver| !waived.iter().any(|finding| waiver.waives(finding)))
        .filter_map(|waiver| {
            let name = waiver.rule.as_ref().ok()?.name();

            Some(Finding {
                line: waiver.line,
                rule: Rule::UnusedWaiver,
                message: format!(
                    "`bs-allow({name})` waives nothing on its line or the next; delete it"
                ),
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

/// `bs-allow(<rule>): <reason>` as its rule name and reason.
fn split_bs_allow(body: &str) -> Option<(&str, &str)> {
    body.strip_prefix("bs-allow(")?.split_once("):")
}

fn bs_allow_problem(comment: &Comment, body: &str) -> Option<(Rule, String)> {
    let malformed = |message: &str| Some((Rule::WaiverNeedsReason, message.to_owned()));

    if comment.kind != CommentKind::Line {
        malformed("only a plain `//` comment waives a rule; this one does nothing")
    } else if let Some((name, reason)) = split_bs_allow(body) {
        rule_problem(name)
            .or_else(|| reason_problem(reason).map(|message| (Rule::WaiverNeedsReason, message)))
    } else {
        malformed("write a waiver as `// bs-allow(<rule>): <reason>`")
    }
}

fn rule_problem(name: &str) -> Option<(Rule, String)> {
    match name.parse::<Rule>() {
        Ok(rule) if rule.is_waivable() => None,
        Ok(_) => Some((
            Rule::WaiverNeedsReason,
            format!("`{name}` guards the gate itself, so it can't be waived; fix the code"),
        )),
        Err(_) if pattern_rule(name) => Some((
            Rule::WaiverNeedsReason,
            format!(
                "`{name}` is a pattern rule; waive it with `// ast-grep-ignore: {name} -- <reason>`"
            ),
        )),
        Err(unknown) => Some((
            Rule::UnknownWaiver,
            format!("{unknown}, so this waiver waives nothing; check the name against the README"),
        )),
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
    let next_is_code = is_code_line(source, comment.line + 1);

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

/// A line that holds code: not blank, and not opening with a comment.
fn is_code_line(source: &Source, number: usize) -> bool {
    !source.is_blank(number) && !source.starts_own_line_comment(number)
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
        let text = "// bs-allow(no-such-rule): there is a reason for this one\nfn key() {}\n// bs-allow(no-boolen-trap): a typo waives nothing at all\nfn draw(filled: bool) {}\n";

        assert_eq!(lines_of(Rule::UnknownWaiver, text), [1, 3]);
        assert_eq!(lines_of(Rule::NoBooleanTrap, text), [4]);
        assert_eq!(waiver_lines(text), NONE);
    }

    #[test]
    fn a_waiver_takes_its_finding_off_the_report() {
        let text = "// bs-allow(no-boolean-trap): the C API this mirrors takes a flag\nfn draw(filled: bool) {}\nfn fill(solid: bool) {} // bs-allow(no-boolean-trap): the C API takes this flag\n";

        assert_eq!(lines_of(Rule::NoBooleanTrap, text), NONE);
        assert_eq!(lines_of(Rule::UnusedWaiver, text), NONE);
    }

    #[test]
    fn a_waiver_that_waives_nothing_is_reported() {
        let text = "// bs-allow(no-boolean-trap): the C API this mirrors takes a flag\nfn draw(fill: Fill) {}\n\n// bs-allow(no-boolean-trap): the C API this mirrors takes a flag\n\nfn fill(solid: bool) {}\n";

        assert_eq!(lines_of(Rule::UnusedWaiver, text), [1, 4]);
        assert_eq!(lines_of(Rule::NoBooleanTrap, text), [6]);
    }

    #[test]
    fn a_waiver_that_doesnt_count_isnt_also_unused() {
        let text = "// bs-allow(no-boolean-trap): fine\nfn draw(fill: Fill) {}\n// bs-allow(no-hidden-code): this include is generated code\nfn other() {}\n";

        assert_eq!(lines_of(Rule::UnusedWaiver, text), NONE);
        assert_eq!(waiver_lines(text), [1, 3]);
    }

    #[test]
    fn a_marker_in_a_string_isnt_a_waiver() {
        let text = "const NOTE: &str = \"// bs-allow(no-boolean-trap): see the docs for this\";\nfn draw(filled: bool) {}\n";

        assert_eq!(lines_of(Rule::NoBooleanTrap, text), [2]);
        assert_eq!(lines_of(Rule::UnusedWaiver, text), NONE);
    }

    #[test]
    fn waivers_are_parsed_once_with_their_rule_and_reason() {
        let source = crate::source::Source::new(
            "src/sample.rs".into(),
            "// bs-allow(complexity): one arm per key\n/// bs-allow(complexity): a doc comment\n// bs-allow(nope): why not\n",
        );
        let parsed: Vec<(usize, bool)> = super::parse(&source)
            .iter()
            .map(|waiver| (waiver.line, waiver.rule.is_ok()))
            .collect();

        assert_eq!(parsed, [(1, true), (3, false)]);
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
