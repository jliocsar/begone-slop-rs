//! `no-needless-main-result`: `main` returns a `Result` only when it can fail.
//!
//! `fn main() -> Result<(), Box<dyn Error>> { ...; Ok(()) }` with no `?` and no other `return`
//! promises a failure that can't happen. Drop the return type and the `Ok(())`. A `?` inside a
//! closure or a nested item doesn't count: it returns from that, not from `main`.

use syn::visit::{self, Visit};
use syn::{
    Block, Expr, ExprAsync, ExprClosure, ExprReturn, ExprTry, Item, ItemFn, ReturnType, Stmt, Type,
};

use super::{FileContext, Finding, Rule};

/// Whether a body can leave with anything but `Ok(())`.
#[derive(Default)]
struct Exits {
    fallible: bool,
}

impl<'ast> Visit<'ast> for Exits {
    fn visit_expr_async(&mut self, _: &'ast ExprAsync) {}

    fn visit_expr_closure(&mut self, _: &'ast ExprClosure) {}

    fn visit_expr_return(&mut self, exit: &'ast ExprReturn) {
        if !exit.expr.as_deref().is_some_and(is_ok_unit) {
            self.fallible = true;
        }

        visit::visit_expr_return(self, exit);
    }

    fn visit_expr_try(&mut self, exit: &'ast ExprTry) {
        self.fallible = true;

        visit::visit_expr_try(self, exit);
    }

    fn visit_item(&mut self, _: &'ast Item) {}
}

fn returns_result(output: &ReturnType) -> bool {
    match output {
        ReturnType::Type(_, returned) => match &**returned {
            Type::Path(path) => path
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "Result"),
            _ => false,
        },
        ReturnType::Default => false,
    }
}

fn is_ok_unit(expression: &Expr) -> bool {
    let Expr::Call(call) = expression else {
        return false;
    };

    let named_ok = matches!(&*call.func, Expr::Path(path) if path.path.is_ident("Ok"));
    let unit = matches!(call.args.first(), Some(Expr::Tuple(tuple)) if tuple.elems.is_empty());

    named_ok && unit && call.args.len() == 1
}

/// Whether the only way out of `body` is the `Ok(())` at its end.
fn only_ever_ok(body: &Block) -> bool {
    let ends_ok = matches!(body.stmts.last(), Some(Stmt::Expr(tail, None)) if is_ok_unit(tail));
    let mut exits = Exits::default();

    exits.visit_block(body);

    ends_ok && !exits.fallible
}

fn needless_result(function: &ItemFn) -> bool {
    function.sig.ident == "main"
        && returns_result(&function.sig.output)
        && only_ever_ok(&function.block)
}

pub(super) fn check(file: &FileContext) -> Vec<Finding> {
    file.syntax
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if needless_result(function) => Some(Finding {
                line: function.sig.ident.span().start().line,
                rule: Rule::NoNeedlessMainResult,
                message: "`main` returns a `Result`, but its only way out is `Ok(())`; drop the return type and the `Ok(())`".to_owned(),
                edit: None,
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::testing::{NONE, lines_of};

    #[test]
    fn a_main_that_only_returns_ok_is_reported() {
        let text =
            "fn main() -> Result<(), Box<dyn Error>> {\n    println!(\"hi\");\n\n    Ok(())\n}\n";

        assert_eq!(lines_of(Rule::NoNeedlessMainResult, text), [1]);
    }

    #[test]
    fn a_main_that_can_fail_passes() {
        let text = "fn main() -> Result<(), Error> {\n    run()?;\n\n    Ok(())\n}\n\nfn other() -> Result<(), Error> {\n    Ok(())\n}\n";

        assert_eq!(lines_of(Rule::NoNeedlessMainResult, text), NONE);
    }

    #[test]
    fn a_question_mark_in_a_closure_doesnt_count() {
        let text = "fn main() -> Result<(), Error> {\n    let parse = |text: &str| -> Result<u8, Error> { Ok(text.parse()?) };\n\n    Ok(())\n}\n";

        assert_eq!(lines_of(Rule::NoNeedlessMainResult, text), [1]);
    }

    #[test]
    fn an_early_error_return_counts() {
        let text = "fn main() -> Result<(), Error> {\n    if broken() {\n        return Err(Error::Broken);\n    }\n\n    Ok(())\n}\n";

        assert_eq!(lines_of(Rule::NoNeedlessMainResult, text), NONE);
    }
}
