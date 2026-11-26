use rust_decimal::Decimal;

use crate::lexer::TfUnit;
use crate::span::Span;

/// An identifier with its source [`Span`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

/// A parsed LangMelogne program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}

/// A statement in a program or `def` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Assign {
        name: Ident,
        value: Expr,
        span: Span,
    },
    Def(Def),
    Error(Span),
}

impl Stmt {
    /// Returns the source [`Span`] covering this statement.
    pub fn span(&self) -> Span {
        match self {
            Stmt::Assign { span, .. } | Stmt::Error(span) => *span,
            Stmt::Def(d) => d.span,
        }
    }
}

/// A function definition (`def`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Def {
    pub name: Ident,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub ret: Expr,
    pub span: Span,
}

/// A parameter in a `def` signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: Ident,
    pub default: Option<Expr>,
    pub span: Span,
}

/// A timeframe literal (`5m`, `1h`, `1d`, `2w`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeframe {
    pub n: u32,
    pub unit: TfUnit,
    pub span: Span,
}

/// Unary arithmetic operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnOp {
    Neg,
    Pos,
}

/// Binary arithmetic operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

impl CmpOp {
    /// Returns the source spelling of this comparison operator.
    pub fn as_str(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::NotEq => "!=",
            CmpOp::Lt => "<",
            CmpOp::LtEq => "<=",
            CmpOp::Gt => ">",
            CmpOp::GtEq => ">=",
        }
    }
}

/// A call argument, positional or keyword (`name = expr`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arg {
    pub name: Option<Ident>,
    pub value: Expr,
    pub span: Span,
}

/// An expression AST node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Num {
        value: Decimal,
        pct: bool,
        span: Span,
    },
    Timeframe(Timeframe),
    Str(String, Span),
    Bool(bool, Span),
    Name(Ident),
    Unary {
        op: UnOp,
        e: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        l: Box<Expr>,
        r: Box<Expr>,
        span: Span,
    },
    Compare {
        first: Box<Expr>,
        rest: Vec<(CmpOp, Expr)>,
        span: Span,
    },
    Not(Box<Expr>, Span),
    And(Box<Expr>, Box<Expr>, Span),
    Or(Box<Expr>, Box<Expr>, Span),
    IfElse {
        cond: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Arg>,
        span: Span,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Attr {
        base: Box<Expr>,
        name: Ident,
        span: Span,
    },
    At {
        base: Box<Expr>,
        tf: Timeframe,
        span: Span,
    },
    Error(Span),
}

impl Expr {
    /// Returns the source [`Span`] covering this expression.
    pub fn span(&self) -> Span {
        match self {
            Expr::Num { span, .. }
            | Expr::Str(_, span)
            | Expr::Bool(_, span)
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Compare { span, .. }
            | Expr::Not(_, span)
            | Expr::And(_, _, span)
            | Expr::Or(_, _, span)
            | Expr::IfElse { span, .. }
            | Expr::Call { span, .. }
            | Expr::Index { span, .. }
            | Expr::Attr { span, .. }
            | Expr::At { span, .. }
            | Expr::Error(span) => *span,
            Expr::Timeframe(tf) => tf.span,
            Expr::Name(id) => id.span,
        }
    }
}

impl Drop for Expr {
    fn drop(&mut self) {
        let mut stack: Vec<Expr> = Vec::new();
        drain_children(self, &mut stack);
        while let Some(mut node) = stack.pop() {
            drain_children(&mut node, &mut stack);
        }
    }
}

fn drain_children(expr: &mut Expr, stack: &mut Vec<Expr>) {
    let dummy = || Expr::Bool(false, Span::empty(0));
    match expr {
        Expr::Unary { e, .. } | Expr::Not(e, _) => {
            stack.push(std::mem::replace(e.as_mut(), dummy()));
        }
        Expr::Binary { l, r, .. } | Expr::And(l, r, _) | Expr::Or(l, r, _) => {
            stack.push(std::mem::replace(l.as_mut(), dummy()));
            stack.push(std::mem::replace(r.as_mut(), dummy()));
        }
        Expr::Compare { first, rest, .. } => {
            stack.push(std::mem::replace(first.as_mut(), dummy()));
            for (_, rhs) in std::mem::take(rest) {
                stack.push(rhs);
            }
        }
        Expr::IfElse {
            cond,
            then,
            otherwise,
            ..
        } => {
            stack.push(std::mem::replace(cond.as_mut(), dummy()));
            stack.push(std::mem::replace(then.as_mut(), dummy()));
            stack.push(std::mem::replace(otherwise.as_mut(), dummy()));
        }
        Expr::Call { callee, args, .. } => {
            stack.push(std::mem::replace(callee.as_mut(), dummy()));
            for arg in std::mem::take(args) {
                stack.push(arg.value);
            }
        }
        Expr::Index { base, index, .. } => {
            stack.push(std::mem::replace(base.as_mut(), dummy()));
            stack.push(std::mem::replace(index.as_mut(), dummy()));
        }
        Expr::Attr { base, .. } | Expr::At { base, .. } => {
            stack.push(std::mem::replace(base.as_mut(), dummy()));
        }
        Expr::Num { .. }
        | Expr::Timeframe(_)
        | Expr::Str(_, _)
        | Expr::Bool(_, _)
        | Expr::Name(_)
        | Expr::Error(_) => {}
    }
}
