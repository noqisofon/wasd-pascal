//! 式のASTノード。
//!
//! 今回のスコープ: リテラル・識別子参照・二項演算・単項演算・括弧のみ。
//! 集合式やポインタ参照(`^`)などUCSD拡張以降で必要になるノードは含めない。

use crate::ident::Identifier;
use crate::span::Span;

/// 式。
///
/// `#[non_exhaustive]`: 将来（配列添字式、集合式、レコードフィールド参照など）
/// バリアントを追加してもワークスペース外からの`match`を壊さないようにするため。
/// 同一クレート内・同一ワークスペース内の他クレート（`wasd-parser`/`wasd-sema`）から
/// 見ても、新しいバリアント追加時にワイルドカードアームなしの`match`はコンパイルエラーに
/// なるので、拡張時の影響範囲を洗い出しやすくなる。
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Expr {
    IntLiteral(i64, Span),
    RealLiteral(f64, Span),
    StringLiteral(String, Span),
    BoolLiteral(bool, Span),
    Identifier(Identifier),
    BinaryOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    UnaryOp {
        op: UnOp,
        operand: Box<Expr>,
        span: Span,
    },
    Paren(Box<Expr>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::IntLiteral(_, span) => *span,
            Expr::RealLiteral(_, span) => *span,
            Expr::StringLiteral(_, span) => *span,
            Expr::BoolLiteral(_, span) => *span,
            Expr::Identifier(ident) => ident.span,
            Expr::BinaryOp { span, .. } => *span,
            Expr::UnaryOp { span, .. } => *span,
            Expr::Paren(_, span) => *span,
        }
    }
}

/// 二項演算子。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    IntDiv,
    Mod,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
}

/// 単項演算子。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

/// `CONST`宣言の右辺に書けるリテラル値。
///
/// `CONST`セクションは今回のスコープでは「識別子 = リテラル」のみを扱い、
/// 任意の定数式（`CONST x = 1 + 2;`のような式）は含めない。そのため`Expr`とは
/// 別に、リテラルのみを表す小さな型として定義する。
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64, Span),
    Real(f64, Span),
    Str(String, Span),
    Bool(bool, Span),
}

impl Literal {
    pub fn span(&self) -> Span {
        match self {
            Literal::Int(_, span) => *span,
            Literal::Real(_, span) => *span,
            Literal::Str(_, span) => *span,
            Literal::Bool(_, span) => *span,
        }
    }
}

impl From<Literal> for Expr {
    fn from(lit: Literal) -> Self {
        match lit {
            Literal::Int(v, span) => Expr::IntLiteral(v, span),
            Literal::Real(v, span) => Expr::RealLiteral(v, span),
            Literal::Str(v, span) => Expr::StringLiteral(v, span),
            Literal::Bool(v, span) => Expr::BoolLiteral(v, span),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_expr_variant_reports_its_span() {
        let s = Span::new(0, 1);
        assert_eq!(Expr::IntLiteral(1, s).span(), s);
        assert_eq!(Expr::RealLiteral(1.0, s).span(), s);
        assert_eq!(Expr::StringLiteral("a".into(), s).span(), s);
        assert_eq!(Expr::BoolLiteral(true, s).span(), s);
        assert_eq!(Expr::Identifier(Identifier::new("x", s)).span(), s);
        assert_eq!(Expr::Paren(Box::new(Expr::IntLiteral(1, s)), s).span(), s);

        let bin = Expr::BinaryOp {
            op: BinOp::Add,
            lhs: Box::new(Expr::IntLiteral(1, s)),
            rhs: Box::new(Expr::IntLiteral(2, s)),
            span: s,
        };
        assert_eq!(bin.span(), s);

        let un = Expr::UnaryOp {
            op: UnOp::Neg,
            operand: Box::new(Expr::IntLiteral(1, s)),
            span: s,
        };
        assert_eq!(un.span(), s);
    }

    #[test]
    fn literal_converts_into_matching_expr_variant() {
        let s = Span::new(0, 1);
        assert_eq!(Expr::from(Literal::Int(42, s)), Expr::IntLiteral(42, s));
        assert_eq!(
            Expr::from(Literal::Str("hi".into(), s)),
            Expr::StringLiteral("hi".into(), s)
        );
    }
}
