//! `no-work-in-format-args`: a formatting macro prints values; it doesn't compute them.
//!
//! `println!("{}", (1..=10).filter(odd).map(square).sum::<i32>() & 0xff)` hides the work
//! inside the print. Compute it into a `let` with a name that says what it is, then print the
//! name. An argument may be a literal, a name, a field, a reference, an index, or a
//! constructor (`Some(..)`, `vec![..]`) of those, and `.display()` on one, which only says how
//! to print it. Any other method call, a function call, a closure, an operator, a cast or a
//! block is work.

use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprLit, Lit, Macro, Token, UnOp};

use super::{FileContext, Finding, Rule};

const FORMAT_MACROS: [&str; 8] = [
    "eprint",
    "eprintln",
    "format",
    "format_args",
    "print",
    "println",
    "write",
    "writeln",
];

struct FormatArguments {
    findings: Vec<Finding>,
}

impl<'ast> Visit<'ast> for FormatArguments {
    fn visit_macro(&mut self, invocation: &'ast Macro) {
        let name = invocation
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();

        if FORMAT_MACROS.contains(&name.as_str()) {
            let arguments = invocation
                .parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
                .unwrap_or_default();

            for argument in values_after_the_template(&arguments) {
                if let Some(work) = work(argument) {
                    self.findings.push(Finding {
                        line: argument.span().start().line,
                        rule: Rule::NoWorkInFormatArgs,
                        message: format!("an argument of `{name}!` is {work}; compute it into a named `let` first, then print the name"),
                        edit: None,
                    });
                }
            }
        }

        visit::visit_macro(self, invocation);
    }
}

/// The values a format string reads: every argument after the template, with `name = value`
/// read as its value.
fn values_after_the_template(arguments: &Punctuated<Expr, Token![,]>) -> Vec<&Expr> {
    arguments
        .iter()
        .skip_while(|argument| !is_string_literal(argument))
        .skip(1)
        .map(|argument| match argument {
            Expr::Assign(named) => &*named.right,
            value => value,
        })
        .collect()
}

fn is_string_literal(argument: &Expr) -> bool {
    matches!(
        argument,
        Expr::Lit(ExprLit {
            lit: Lit::Str(_),
            ..
        })
    )
}

/// What makes `argument` work, or none when it only names or builds a value.
fn work(argument: &Expr) -> Option<&'static str> {
    match argument {
        Expr::Lit(_) | Expr::Path(_) | Expr::Macro(_) => None,
        Expr::Field(field) => work(&field.base),
        Expr::Reference(reference) => work(&reference.expr),
        Expr::Paren(inner) => work(&inner.expr),
        Expr::Group(inner) => work(&inner.expr),
        Expr::Index(index) => work(&index.expr).or_else(|| work(&index.index)),
        Expr::Unary(unary) => unary_work(&unary.op, &unary.expr),
        Expr::Tuple(tuple) => tuple.elems.iter().find_map(work),
        Expr::Array(array) => array.elems.iter().find_map(work),
        Expr::Call(call) => call_work(call),
        Expr::MethodCall(call) if call.method == "display" && call.args.is_empty() => {
            work(&call.receiver)
        }
        other => Some(kind_of_work(other)),
    }
}

fn unary_work(operator: &UnOp, operand: &Expr) -> Option<&'static str> {
    match operator {
        UnOp::Deref(_) | UnOp::Neg(_) => work(operand),
        _ => Some("an operator"),
    }
}

/// A call to a constructor (`Some(..)`, `Point(..)`) builds a value; any other call is work.
fn call_work(call: &ExprCall) -> Option<&'static str> {
    let constructor = match &*call.func {
        Expr::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| {
                segment
                    .ident
                    .to_string()
                    .starts_with(char::is_uppercase)
            }),
        _ => false,
    };

    if constructor {
        call.args.iter().find_map(work)
    } else {
        Some("a function call")
    }
}

fn kind_of_work(argument: &Expr) -> &'static str {
    match argument {
        Expr::MethodCall(_) => "a method call",
        Expr::Closure(_) => "a closure",
        Expr::Binary(_) => "an operator",
        Expr::Cast(_) => "a cast",
        Expr::If(_) | Expr::Match(_) => "a branch",
        Expr::Block(_) | Expr::Unsafe(_) => "a block",
        Expr::Range(_) => "a range",
        Expr::Try(_) => "a `?`",
        Expr::Await(_) => "an `.await`",
        _ => "an expression",
    }
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    let mut arguments = FormatArguments {
        findings: Vec::new(),
    };

    arguments.visit_file(file.syntax);

    arguments.findings
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn work_inside_a_print_is_reported() {
        let text = "fn main() {\n    println!(\"{}\", numbers.iter().sum::<i32>());\n    let line = format!(\"{} {}\", width * 2, height as u64);\n    eprintln!(\"{}\", read(path));\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), [2, 3, 3, 4]);
    }

    #[test]
    fn named_values_and_constructors_pass() {
        let text = "fn main() {\n    let low_byte = total & 0xff;\n\n    println!(\"{low_byte} {:?} {:?} {}\", Some(vec![1, 2, 3]), 'a', point.x);\n    println!(\"{} {}\", &names[0], -offset);\n    eprintln!(\"{}: {}\", report.path.display(), finding.line);\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), NONE);
    }

    #[test]
    fn the_destination_of_a_write_isnt_an_argument() {
        let text = "fn show(formatter: &mut Formatter) -> fmt::Result {\n    write!(formatter.inner(), \"{}\", self.name)?;\n    writeln!(formatter, \"{count}\", count = items.len())\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), [3]);
    }

    #[test]
    fn work_in_a_constructor_is_still_work() {
        let text = "fn main() {\n    println!(\"{:?}\", Some(name.trim()));\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), [2]);
    }
}
