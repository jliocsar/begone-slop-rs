//! `max-lines`: a file over 1000 lines is sprawling.
//!
//! High on purpose, as in the TypeScript preset: a low cap doesn't shrink code, it moves it
//! into modules that exist only to sit under the cap. Functions have clippy's
//! `too_many_lines`.

use super::{Finding, Rule};
use crate::source::Source;

const MAX_LINES: usize = 1000;

pub fn check(source: &Source) -> Option<Finding> {
    let count = source.lines.len();

    (count > MAX_LINES).then(|| Finding {
        line: MAX_LINES + 1,
        rule: Rule::MaxLines,
        message: format!("this file has {count} lines (max {MAX_LINES}); split it by concern"),
        edit: None,
    })
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::lines_of;

    #[test]
    fn a_long_file_is_reported_where_it_crosses_the_cap() {
        let text = "const LIMIT: u8 = 1;\n".repeat(1001);

        assert_eq!(lines_of(Rule::MaxLines, &text), [1001]);
    }
}
