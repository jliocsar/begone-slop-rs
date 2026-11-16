//! A source file as lines, which is what the padding rules read and their fixes rewrite, plus
//! its comments, which syn throws away.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// A waiver's reason is a sentence: at least this many words.
pub const MIN_REASON_WORDS: usize = 3;

/// A line-based change one finding asks for. Lines are 1-based, as spans and editors count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    InsertBlankAfter(usize),
    DeleteLines(Vec<usize>),
    /// Lines `first..=last` become `lines`.
    Replace {
        first: usize,
        last: usize,
        lines: Vec<String>,
    },
}

/// How a comment opens: `//`, `/*`, or a doc comment (`///`, `//!`, `/**`, `/*!`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentKind {
    Block,
    Doc,
    Line,
}

/// One comment: the line it starts on, its kind, its text without the markers, and whether
/// code comes before it on that line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub line: usize,
    pub kind: CommentKind,
    pub text: String,
    pub after_code: bool,
}

/// A Rust source file, split into lines, with the comments a lexer found in it.
pub struct Source {
    pub path: PathBuf,
    pub lines: Vec<String>,
    comments: Vec<Comment>,
    line_break: &'static str,
    trailing_newline: bool,
}

/// Walks source text one character at a time, keeping the line count, so comments are told
/// apart from string and character literals that look like them.
struct Lexer {
    characters: Vec<char>,
    at: usize,
    line: usize,
    line_has_code: bool,
    comments: Vec<Comment>,
}

impl Source {
    #[must_use]
    pub fn new(path: PathBuf, text: &str) -> Self {
        let line_break = if text.contains("\r\n") { "\r\n" } else { "\n" };

        Self {
            path,
            lines: text
                .lines()
                .map(str::to_owned)
                .collect(),
            comments: Lexer::comments(text),
            line_break,
            trailing_newline: text.ends_with('\n'),
        }
    }

    #[must_use]
    pub fn text(&self) -> String {
        let mut text = self.lines.join(self.line_break);

        if self.trailing_newline {
            text.push_str(self.line_break);
        }

        text
    }

    #[must_use]
    pub fn line(&self, number: usize) -> Option<&str> {
        number
            .checked_sub(1)
            .and_then(|index| self.lines.get(index))
            .map(String::as_str)
    }

    #[must_use]
    pub fn is_blank(&self, number: usize) -> bool {
        self.line(number)
            .is_none_or(|text| text.trim().is_empty())
    }

    /// Every comment in the file, in order, told apart from string and character literals
    /// that look like one. This is the only place the rules find comments.
    #[must_use]
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// Whether a comment opens on line `number` with nothing but whitespace before it.
    #[must_use]
    pub fn starts_own_line_comment(&self, number: usize) -> bool {
        self.comments
            .iter()
            .any(|comment| comment.line == number && !comment.after_code)
    }

    /// Rewrites lines as `edits` ask, then lexes the comments again: the next pass reads
    /// them at their new lines.
    pub fn apply(&mut self, edits: &[Edit]) {
        let mut inserts: BTreeSet<usize> = BTreeSet::new();
        let mut deletes: BTreeSet<usize> = BTreeSet::new();
        let mut replacements: BTreeMap<usize, &[String]> = BTreeMap::new();

        for edit in edits {
            match edit {
                Edit::InsertBlankAfter(number) => {
                    inserts.insert(*number);
                }
                Edit::DeleteLines(numbers) => deletes.extend(numbers.iter().copied()),
                Edit::Replace { first, last, lines } => {
                    deletes.extend(*first..=*last);
                    replacements.insert(*first, lines);
                }
            }
        }

        self.lines = std::mem::take(&mut self.lines)
            .into_iter()
            .zip(1..)
            .flat_map(|(text, number)| {
                let replaced = replacements
                    .get(&number)
                    .copied()
                    .unwrap_or_default();
                let kept = (!deletes.contains(&number)).then_some(text);
                let blank = inserts
                    .contains(&number)
                    .then(String::new);

                replaced
                    .iter()
                    .cloned()
                    .chain(kept)
                    .chain(blank)
            })
            .collect();
        self.comments = Lexer::comments(&self.text());
    }
}

impl Lexer {
    fn comments(text: &str) -> Vec<Comment> {
        let mut lexer = Self {
            characters: text.chars().collect(),
            at: 0,
            line: 1,
            line_has_code: false,
            comments: Vec::new(),
        };

        while let Some(character) = lexer.peek(0) {
            match (character, lexer.peek(1)) {
                ('/', Some('/')) => lexer.line_comment(),
                ('/', Some('*')) => lexer.block_comment(),
                ('"', _) => lexer.string(),
                ('\'', _) => lexer.quote(),
                _ if character.is_alphabetic() || character == '_' => lexer.word(),
                _ => {
                    lexer.code(character);
                    lexer.at += 1;
                }
            }
        }

        lexer.comments
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.characters
            .get(self.at + ahead)
            .copied()
    }

    /// Steps over one character that isn't in a comment.
    fn code(&mut self, character: char) {
        if character == '\n' {
            self.line += 1;
            self.line_has_code = false;
        } else if !character.is_whitespace() {
            self.line_has_code = true;
        }
    }

    fn text_until(&self, end: usize) -> String {
        self.characters
            .get(self.at..end)
            .unwrap_or_default()
            .iter()
            .collect()
    }

    fn line_comment(&mut self) {
        self.at += 2;

        let end = (self.at..self.characters.len())
            .find(|index| self.characters.get(*index) == Some(&'\n'))
            .unwrap_or(self.characters.len());
        let body = self.text_until(end);
        let doc = (body.starts_with('/') && !body.starts_with("//")) || body.starts_with('!');

        self.comments.push(Comment {
            line: self.line,
            kind: if doc {
                CommentKind::Doc
            } else {
                CommentKind::Line
            },
            text: if doc {
                body.get(1..)
                    .unwrap_or_default()
                    .to_owned()
            } else {
                body
            },
            after_code: self.line_has_code,
        });
        self.at = end;
    }

    fn block_comment(&mut self) {
        let line = self.line;
        let after_code = self.line_has_code;

        self.at += 2;

        let doc = matches!(
            (self.peek(0), self.peek(1)),
            (Some('*'), Some(next)) if next != '*' && next != '/'
        ) || self.peek(0) == Some('!');
        let start = self.at;
        let mut depth = 1;

        while depth > 0
            && let Some(character) = self.peek(0)
        {
            match (character, self.peek(1)) {
                ('*', Some('/')) => {
                    depth -= 1;
                    self.at += 2;
                }
                ('/', Some('*')) => {
                    depth += 1;
                    self.at += 2;
                }
                _ => {
                    self.line += usize::from(character == '\n');
                    self.at += 1;
                }
            }
        }

        let end = self.at.saturating_sub(2).max(start);

        self.comments.push(Comment {
            line,
            kind: if doc {
                CommentKind::Doc
            } else {
                CommentKind::Block
            },
            text: self
                .characters
                .get(start..end)
                .unwrap_or_default()
                .iter()
                .collect(),
            after_code,
        });
    }

    /// A `"..."` string, escapes included.
    fn string(&mut self) {
        self.line_has_code = true;
        self.at += 1;

        while let Some(character) = self.peek(0) {
            self.at += 1;

            match character {
                '\\' => {
                    self.line += usize::from(self.peek(0) == Some('\n'));
                    self.at += 1;
                }
                '"' => return,
                '\n' => self.line += 1,
                _ => {}
            }
        }
    }

    /// `r"..."`, `r#"..."#` and their `b`/`c` forms, from the opening quote.
    fn raw_string(&mut self) {
        let hashes = self.hashes_from(0);

        self.at += hashes + 1;

        while let Some(character) = self.peek(0) {
            self.at += 1;
            self.line += usize::from(character == '\n');

            if character == '"' && (0..hashes).all(|ahead| self.peek(ahead) == Some('#')) {
                self.at += hashes;

                return;
            }
        }
    }

    /// A character literal (`'x'`, `'\n'`), or a lifetime or label (`'a`), which has no end.
    fn quote(&mut self) {
        self.line_has_code = true;

        if self.peek(1) == Some('\\') {
            self.at += 2;

            while let Some(character) = self.peek(0) {
                self.at += 1;

                if character == '\'' {
                    return;
                }
            }
        } else if self.peek(2) == Some('\'') {
            self.at += 3;
        } else {
            self.at += 1;
        }
    }

    /// An identifier or keyword, which may open a raw string (`r"`, `br#"`) or a byte or C
    /// string (`b"`, `c"`, `b'x'`).
    fn word(&mut self) {
        self.line_has_code = true;

        let start = self.at;

        while self
            .peek(0)
            .is_some_and(|character| character.is_alphanumeric() || character == '_')
        {
            self.at += 1;
        }

        let word = self.text_until_from(start);
        let raw = matches!(word.as_str(), "r" | "br" | "cr");
        let opens_raw = raw
            && match self.peek(0) {
                Some('"') => true,
                Some('#') => self.peek(self.hashes_from(0)) == Some('"'),
                _ => false,
            };

        if opens_raw {
            self.raw_string();
        } else if matches!(word.as_str(), "b" | "c") && self.peek(0) == Some('"') {
            self.string();
        } else if word == "b" && self.peek(0) == Some('\'') {
            self.quote();
        }
    }

    /// How many `#` follow, starting `ahead` characters on.
    fn hashes_from(&self, ahead: usize) -> usize {
        self.characters
            .iter()
            .skip(self.at + ahead)
            .take_while(|character| **character == '#')
            .count()
    }

    fn text_until_from(&self, start: usize) -> String {
        self.characters
            .get(start..self.at)
            .unwrap_or_default()
            .iter()
            .collect()
    }
}

/// A waiver's reason is a sentence: [`MIN_REASON_WORDS`] words or more, not `x` or `fine`.
#[must_use]
pub fn is_real_reason(reason: &str) -> bool {
    reason
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .count()
        >= MIN_REASON_WORDS
}

#[cfg(test)]
mod tests {
    use super::{CommentKind, Edit, Source};

    fn source(text: &str) -> Source {
        Source::new("sample.rs".into(), text)
    }

    #[test]
    fn comments_are_told_apart_from_literals() {
        let text = "let url = \"http://x\"; // trailing\nlet raw = r#\"/* not */ \"//\"#;\nlet quote = '\"'; /* block\nspans */\n/// doc\nfn tick<'a>(x: &'a str) {} // last\n";
        let found: Vec<(usize, CommentKind, String, bool)> = source(text)
            .comments()
            .iter()
            .map(|comment| {
                (
                    comment.line,
                    comment.kind,
                    comment.text.clone(),
                    comment.after_code,
                )
            })
            .collect();

        assert_eq!(
            found,
            [
                (1, CommentKind::Line, " trailing".to_owned(), true),
                (3, CommentKind::Block, " block\nspans ".to_owned(), true),
                (5, CommentKind::Doc, " doc".to_owned(), false),
                (6, CommentKind::Line, " last".to_owned(), true),
            ]
        );
    }

    #[test]
    fn edits_insert_and_delete_whole_lines() {
        let mut sample = source("one\ntwo\n\nthree\n");

        sample.apply(&[Edit::InsertBlankAfter(1), Edit::DeleteLines(vec![3])]);

        assert_eq!(sample.text(), "one\n\ntwo\nthree\n");
    }

    #[test]
    fn comments_move_with_the_lines_an_edit_moves() {
        let mut sample = source("one();\n// note\ntwo();\n");

        sample.apply(&[Edit::InsertBlankAfter(1)]);

        let lines: Vec<usize> = sample
            .comments()
            .iter()
            .map(|comment| comment.line)
            .collect();

        assert_eq!(lines, [3]);
    }

    #[test]
    fn crlf_files_keep_their_line_breaks() {
        let mut sample = source("one\r\ntwo\r\n");

        sample.apply(&[Edit::InsertBlankAfter(1)]);

        assert_eq!(sample.text(), "one\r\n\r\ntwo\r\n");
    }
}
