//! Statements and items reduced to what the padding rules judge: a line range and a kind.

use syn::spanned::Spanned;
use syn::{Block, Expr, ImplItem, Item, Local, Macro, MacroDelimiter, Stmt, TraitItem, Visibility};

use super::gap::Gap;

const ASSERT_MACROS: [&str; 7] = [
    "assert",
    "assert_eq",
    "assert_ne",
    "assert_matches",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Assert,
    BlockLike,
    Const {
        public: bool,
    },
    ItemBlock,
    /// A `let` that doesn't own a block of its own.
    Let,
    Other,
    Return,
    /// A block's value: its last expression, with no `;` after it.
    Tail,
    Use,
}

/// Whether a unit fits on one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Height {
    OneLine,
    MultiLine,
}

/// One statement or item, from its first attribute to its last token.
#[derive(Clone, Copy, Debug)]
pub(super) struct Unit {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

impl Unit {
    pub(super) fn gap_to(self, next: Self) -> Gap {
        Gap {
            after: self.end,
            before: next.start,
        }
    }

    pub(super) fn of_stmt(stmt: &Stmt) -> Self {
        let (start, end) = lines_of(stmt);

        let height = if end > start {
            Height::MultiLine
        } else {
            Height::OneLine
        };

        let kind = match stmt {
            Stmt::Local(local) => local_kind(local, height),
            Stmt::Item(item) => item_kind(item, height),
            Stmt::Expr(expr, _) => expr_kind(expr, height),
            Stmt::Macro(statement) => statement_macro_kind(&statement.mac, height),
        };

        Self { start, end, kind }
    }

    /// A block's statements, with its value, the last expression with no `;`, as a
    /// [`Kind::Tail`] unless it's a block or a `return` already.
    pub(super) fn of_block(block: &Block) -> Vec<Self> {
        let mut units: Vec<Self> = block.stmts.iter().map(Self::of_stmt).collect();

        if let (Some(last), Some(unit)) = (block.stmts.last(), units.last_mut())
            && is_value(last)
            && unit.kind == Kind::Other
        {
            unit.kind = Kind::Tail;
        }

        units
    }

    pub(super) fn of_item(item: &Item) -> Self {
        let (start, end) = lines_of(item);

        Self {
            start,
            end,
            kind: item_kind(
                item,
                if end > start {
                    Height::MultiLine
                } else {
                    Height::OneLine
                },
            ),
        }
    }

    pub(super) fn of_impl_item(item: &ImplItem) -> Self {
        let (start, end) = lines_of(item);

        let height = if end > start {
            Height::MultiLine
        } else {
            Height::OneLine
        };

        let kind = match item {
            ImplItem::Fn(_) => Kind::ItemBlock,
            ImplItem::Const(constant) if height == Height::OneLine => Kind::Const {
                public: is_public(&constant.vis),
            },
            _ if height == Height::MultiLine => Kind::ItemBlock,
            _ => Kind::Other,
        };

        Self { start, end, kind }
    }

    pub(super) fn of_trait_item(item: &TraitItem) -> Self {
        let (start, end) = lines_of(item);

        let height = if end > start {
            Height::MultiLine
        } else {
            Height::OneLine
        };

        let kind = match item {
            TraitItem::Fn(function) if function.default.is_some() => Kind::ItemBlock,
            TraitItem::Const(_) if height == Height::OneLine => Kind::Const { public: true },
            _ if height == Height::MultiLine => Kind::ItemBlock,
            _ => Kind::Other,
        };

        Self { start, end, kind }
    }
}

fn lines_of(node: &impl Spanned) -> (usize, usize) {
    let span = node.span();

    (span.start().line, span.end().line)
}

fn is_public(visibility: &Visibility) -> bool {
    !matches!(visibility, Visibility::Inherited)
}

fn owns_block(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Async(_)
            | Expr::Block(_)
            | Expr::Const(_)
            | Expr::ForLoop(_)
            | Expr::If(_)
            | Expr::Loop(_)
            | Expr::Match(_)
            | Expr::TryBlock(_)
            | Expr::Unsafe(_)
            | Expr::While(_)
    )
}

/// Whether `expr` starts with a block: `match x { .. }.run()` and `async { .. }.await` are
/// blocks with a call or an `.await` hung off the end.
fn leads_with_block(expr: &Expr) -> bool {
    let mut head = expr;

    loop {
        head = match head {
            Expr::Await(awaited) => &awaited.base,
            Expr::Field(field) => &field.base,
            Expr::MethodCall(call) => &call.receiver,
            Expr::Try(tried) => &tried.expr,
            _ => return owns_block(head),
        };
    }
}

fn is_block_closure(expr: &Expr) -> bool {
    matches!(expr, Expr::Closure(closure) if matches!(*closure.body, Expr::Block(_)))
}

/// Whether the statement, when it's last, is the block's value.
fn is_value(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expr(_, semicolon) => semicolon.is_none(),
        Stmt::Macro(statement) => {
            statement.semi_token.is_none()
                && !matches!(statement.mac.delimiter, MacroDelimiter::Brace(_))
        }
        Stmt::Local(_) | Stmt::Item(_) => false,
    }
}

fn expr_kind(expr: &Expr, height: Height) -> Kind {
    match expr {
        Expr::Return(_) => Kind::Return,
        Expr::Macro(invocation) => macro_kind(&invocation.mac),
        Expr::Assign(assign)
            if height == Height::MultiLine
                && (owns_block(&assign.right) || is_block_closure(&assign.right)) =>
        {
            Kind::BlockLike
        }
        _ if leads_with_block(expr) => Kind::BlockLike,
        _ => Kind::Other,
    }
}

fn local_kind(local: &Local, height: Height) -> Kind {
    let block_like = local.init.as_ref().is_some_and(|init| {
        init.diverge.is_some() || leads_with_block(&init.expr) || is_block_closure(&init.expr)
    });

    if height == Height::MultiLine && block_like {
        Kind::BlockLike
    } else {
        Kind::Let
    }
}

fn item_kind(item: &Item, height: Height) -> Kind {
    match item {
        Item::Use(_) => Kind::Use,
        Item::Const(constant) if height == Height::OneLine => Kind::Const {
            public: is_public(&constant.vis),
        },
        Item::Static(constant) if height == Height::OneLine => Kind::Const {
            public: is_public(&constant.vis),
        },
        Item::Fn(_) | Item::Impl(_) | Item::Trait(_) => Kind::ItemBlock,
        Item::Mod(module) if module.content.is_some() => Kind::ItemBlock,
        _ if height == Height::MultiLine => Kind::ItemBlock,
        _ => Kind::Other,
    }
}

/// A macro written as a statement. One with a braced body over several lines
/// (`tokio::select! { .. }`) holds blocks, so it stands apart like one.
fn statement_macro_kind(invocation: &Macro, height: Height) -> Kind {
    let braced = matches!(invocation.delimiter, MacroDelimiter::Brace(_));

    if braced && height == Height::MultiLine {
        Kind::BlockLike
    } else {
        macro_kind(invocation)
    }
}

fn macro_kind(invocation: &Macro) -> Kind {
    let is_assert = invocation
        .path
        .segments
        .last()
        .is_some_and(|segment| ASSERT_MACROS.iter().any(|name| segment.ident == name));

    if is_assert { Kind::Assert } else { Kind::Other }
}
