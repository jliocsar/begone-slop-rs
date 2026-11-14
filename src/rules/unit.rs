//! Statements and items reduced to what the padding rules judge: a line range and a kind.

use syn::spanned::Spanned;
use syn::{Expr, ImplItem, Item, Local, Macro, Stmt, TraitItem, Visibility};

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
    Const { public: bool },
    ItemBlock,
    Other,
    Return,
    Use,
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
        let multi_line = end > start;

        let kind = match stmt {
            Stmt::Local(local) => local_kind(local, multi_line),
            Stmt::Item(item) => item_kind(item, multi_line),
            Stmt::Expr(expr, _) => expr_kind(expr, multi_line),
            Stmt::Macro(statement) => macro_kind(&statement.mac),
        };

        Self { start, end, kind }
    }

    pub(super) fn of_item(item: &Item) -> Self {
        let (start, end) = lines_of(item);

        Self {
            start,
            end,
            kind: item_kind(item, end > start),
        }
    }

    pub(super) fn of_impl_item(item: &ImplItem) -> Self {
        let (start, end) = lines_of(item);
        let multi_line = end > start;

        let kind = match item {
            ImplItem::Fn(_) => Kind::ItemBlock,
            ImplItem::Const(constant) if !multi_line => Kind::Const {
                public: is_public(&constant.vis),
            },
            _ if multi_line => Kind::ItemBlock,
            _ => Kind::Other,
        };

        Self { start, end, kind }
    }

    pub(super) fn of_trait_item(item: &TraitItem) -> Self {
        let (start, end) = lines_of(item);
        let multi_line = end > start;

        let kind = match item {
            TraitItem::Fn(function) if function.default.is_some() => Kind::ItemBlock,
            TraitItem::Const(_) if !multi_line => Kind::Const { public: true },
            _ if multi_line => Kind::ItemBlock,
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

fn is_block_closure(expr: &Expr) -> bool {
    matches!(expr, Expr::Closure(closure) if matches!(*closure.body, Expr::Block(_)))
}

fn expr_kind(expr: &Expr, multi_line: bool) -> Kind {
    match expr {
        Expr::Return(_) => Kind::Return,
        Expr::Macro(invocation) => macro_kind(&invocation.mac),
        Expr::Assign(assign) if multi_line && owns_block(&assign.right) => Kind::BlockLike,
        _ if owns_block(expr) => Kind::BlockLike,
        _ => Kind::Other,
    }
}

fn local_kind(local: &Local, multi_line: bool) -> Kind {
    let block_like = local.init.as_ref().is_some_and(|init| {
        init.diverge.is_some() || owns_block(&init.expr) || is_block_closure(&init.expr)
    });

    if multi_line && block_like {
        Kind::BlockLike
    } else {
        Kind::Other
    }
}

fn item_kind(item: &Item, multi_line: bool) -> Kind {
    match item {
        Item::Use(_) => Kind::Use,
        Item::Const(constant) if !multi_line => Kind::Const {
            public: is_public(&constant.vis),
        },
        Item::Static(constant) if !multi_line => Kind::Const {
            public: is_public(&constant.vis),
        },
        Item::Fn(_) | Item::Impl(_) | Item::Trait(_) => Kind::ItemBlock,
        Item::Mod(module) if module.content.is_some() => Kind::ItemBlock,
        _ if multi_line => Kind::ItemBlock,
        _ => Kind::Other,
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
