//! `no-work-in-format-args`: a formatting macro prints values; it doesn't compute them.
//!
//! `println!("{}", (1..=10).filter(odd).map(square).sum::<i32>() & 0xff)` hides the work
//! inside the print. Compute it into a `let` with a name that says what it is, then print the
//! name. An argument may be a literal, a name, a field, a reference, an index, a range, or a
//! constructor (`Some(..)`, `vec![..]`) of those, and `.display()` on one, which only says how
//! to print it. Any other method call, a function call, a closure, an operator, a cast or a
//! block is work.
//!
//! Every macro that formats is held to it: the `print`/`write`/`format` family, `panic!`,
//! `anyhow!`, `bail!`, the message of `assert!`, `assert_eq!`, `assert_ne!` (and their `debug_`
//! forms) and `ensure!`, and the `tracing` and `log` macros `trace!`, `debug!`, `info!`,
//! `warn!` and `error!`, whose fields are printed too (`count = rows.len()`, `%path`,
//! `?state`). What an assertion checks isn't printed: it's the test, so it may compute, and so
//! may a `write!` destination. A tracing macro's `target:`/`parent:`/`name:` say where the event
//! goes, not what it prints. Format macros nested in these macros' arguments are checked too.

use syn::ext::IdentExt;
use syn::parse::ParseStream;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprLit, Ident, Lit, LitStr, Macro, Token, UnOp};

use super::{FileContext, Finding, Rule};

/// Every macro the rule reads, by its last path segment.
const LAYOUTS: [(&str, Layout); 23] = [
    ("anyhow", Layout::Template { leading: 0 }),
    ("bail", Layout::Template { leading: 0 }),
    ("eprint", Layout::Template { leading: 0 }),
    ("eprintln", Layout::Template { leading: 0 }),
    ("format", Layout::Template { leading: 0 }),
    ("format_args", Layout::Template { leading: 0 }),
    ("panic", Layout::Template { leading: 0 }),
    ("print", Layout::Template { leading: 0 }),
    ("println", Layout::Template { leading: 0 }),
    ("assert", Layout::Template { leading: 1 }),
    ("debug_assert", Layout::Template { leading: 1 }),
    ("ensure", Layout::Template { leading: 1 }),
    ("write", Layout::Template { leading: 1 }),
    ("writeln", Layout::Template { leading: 1 }),
    ("assert_eq", Layout::Template { leading: 2 }),
    ("assert_ne", Layout::Template { leading: 2 }),
    ("debug_assert_eq", Layout::Template { leading: 2 }),
    ("debug_assert_ne", Layout::Template { leading: 2 }),
    ("debug", Layout::Fields),
    ("error", Layout::Fields),
    ("info", Layout::Fields),
    ("trace", Layout::Fields),
    ("warn", Layout::Fields),
];

/// Where a macro's printed values sit among its arguments.
#[derive(Clone, Copy)]
enum Layout {
    /// A template string after `leading` arguments that aren't printed as written (a `write!`
    /// destination, what an assertion checks), then the values the template reads.
    Template { leading: usize },
    /// `tracing`'s fields, its message and the message's values: everything is printed.
    Fields,
}

struct FormatArguments {
    findings: Vec<Finding>,
}

impl FormatArguments {
    fn check(&mut self, name: &str, layout: Layout, arguments: &[Option<Expr>]) {
        for value in printed(arguments, layout) {
            if let Some(work) = work(value) {
                self.findings.push(Finding {
                    line: value.span().start().line,
                    rule: Rule::NoWorkInFormatArgs,
                    message: format!("an argument of `{name}!` is {work}; compute it into a named `let` first, then print the name"),
                    edit: None,
                });
            }
        }

        // syn leaves macro arguments as tokens, so a `format!` inside them is only seen here.
        let mut nested = Self {
            findings: Vec::new(),
        };

        for argument in arguments.iter().flatten() {
            nested.visit_expr(argument);
        }

        self.findings
            .append(&mut nested.findings);
    }
}

impl<'ast> Visit<'ast> for FormatArguments {
    fn visit_macro(&mut self, invocation: &'ast Macro) {
        let name = invocation
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();
        let layout = LAYOUTS
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, layout)| *layout);

        if let Some(layout) = layout {
            let arguments: Vec<Option<Expr>> = invocation
                .parse_body_with(|input: ParseStream<'_>| {
                    Punctuated::<Option<Expr>, Token![,]>::parse_terminated_with(input, argument)
                })
                .map(|parsed| parsed.into_iter().collect())
                .unwrap_or_default();

            self.check(&name, layout, &arguments);
        }

        visit::visit_macro(self, invocation);
    }
}

/// One argument's value: `name = value` and tracing's `field.path = %value`, `?value` read as
/// their value. None for `target: ..` and the like, which aren't printed.
fn argument(input: ParseStream<'_>) -> syn::Result<Option<Expr>> {
    if input.peek(Ident::peek_any) && input.peek2(Token![:]) && !input.peek2(Token![::]) {
        input.call(Ident::parse_any)?;
        input.parse::<Token![:]>()?;
        input.parse::<Expr>()?;

        return Ok(None);
    }

    let named = input.fork();

    if field_name(&named).is_ok()
        && named.peek(Token![=])
        && !named.peek(Token![==])
        && !named.peek(Token![=>])
    {
        field_name(input)?;
        input.parse::<Token![=]>()?;
    }

    if input.peek(Token![%]) {
        input.parse::<Token![%]>()?;
    } else if input.peek(Token![?]) {
        input.parse::<Token![?]>()?;
    }

    input.parse().map(Some)
}

/// `count`, `"count"` or tracing's dotted `request.path`.
fn field_name(input: ParseStream<'_>) -> syn::Result<()> {
    if input.peek(LitStr) {
        input.parse::<LitStr>()?;
    } else {
        input.call(Ident::parse_any)?;
    }

    while input.peek(Token![.]) {
        input.parse::<Token![.]>()?;
        input.call(Ident::parse_any)?;
    }

    Ok(())
}

/// The values a macro prints: for a template, every argument after it.
fn printed(arguments: &[Option<Expr>], layout: Layout) -> Vec<&Expr> {
    let values = match layout {
        Layout::Fields => arguments,
        Layout::Template { leading } => match arguments.get(leading) {
            Some(Some(template)) if is_string_literal(template) => arguments
                .get(leading + 1..)
                .unwrap_or_default(),
            _ => &[],
        },
    };

    values.iter().flatten().collect()
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
        Expr::Range(range) => [&range.start, &range.end]
            .into_iter()
            .flatten()
            .find_map(|bound| work(bound)),
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
        let text = "fn main() {\n    let low_byte = total & 0xff;\n\n    println!(\"{low_byte} {:?} {:?} {}\", Some(vec![1, 2, 3]), 'a', point.x);\n    println!(\"{} {} {:?}\", &names[0], -offset, &names[..8]);\n    eprintln!(\"{}: {}\", report.path.display(), finding.line);\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), NONE);
    }

    #[test]
    fn the_destination_of_a_write_isnt_an_argument() {
        let text = "fn show(formatter: &mut Formatter) -> fmt::Result {\n    write!(formatter.inner(), \"{}\", self.name)?;\n    writeln!(formatter, \"{count}\", count = items.len())\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), [3]);
    }

    #[test]
    fn work_in_a_constructor_is_still_work() {
        let text = "fn main() {\n    println!(\"{:?}\", Some(name.trim()));\n    println!(\"{:?}\", &names[..count()]);\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), [2, 3]);
    }

    #[test]
    fn tracing_fields_and_message_values_are_arguments() {
        let text = "fn load() {\n    tracing::info!(count = rows.len(), \"loaded\");\n    warn!(%error.kind(), \"failed\");\n    debug!(?self.parse(), \"parsed\");\n    error!(target: \"wyrm\", \"lost {}\", id.to_string());\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), [2, 3, 4, 5]);
    }

    #[test]
    fn tracing_names_fields_and_metadata_pass() {
        let text = "fn load() {\n    info!(target: \"wyrm\", parent: span.id(), %error, path = %path.display(), ?state, block.id = id.0, \"loaded {count}\", count = rows);\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), NONE);
    }

    #[test]
    fn what_an_assertion_checks_isnt_printed_but_its_message_is() {
        let text = "fn check() {\n    assert!(rows.is_empty(), \"left over: {}\", rows.len());\n    assert_eq!(\"wyrm\", name.trim());\n    assert_ne!(parse(text), None, \"{}\", text.trim());\n    ensure!(width > 0, \"width {}\", width * 2);\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), [2, 4, 5]);
    }

    #[test]
    fn panics_and_errors_are_formats() {
        let text = "fn fail() -> Result<()> {\n    let error = anyhow!(\"no {}\", name.trim());\n    panic!(\"bad {}\", value as u8);\n    bail!(\"gone: {}\", read(path)?)\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), [2, 3, 4]);
    }

    #[test]
    fn a_format_nested_in_another_macros_arguments_is_checked() {
        let text = "fn check() {\n    assert!(ready, \"{}\", format!(\"{}\", name.trim()));\n}\n";

        assert_eq!(lines_of(Rule::NoWorkInFormatArgs, text), [2]);
    }
}
