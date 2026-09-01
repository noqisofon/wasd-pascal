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
    /// UCSD拡張の16進数リテラル（`$FF`）に由来する整数式。
    ///
    /// # 設計判断: `IntLiteral`と別バリアントに分ける理由
    ///
    /// レキサ・パーサーはdialectに関わらず`$FF`のような16進数リテラルを
    /// 常に受理する（`wasd_ast::Dialect`のドキュメント参照）。dialectチェック
    /// （ISO 7185では使用不可）は`wasd-sema`が行うが、そのためには
    /// 「このリテラルが`$FF`という16進数表記で書かれていた」という情報が
    /// 意味解析の時点まで残っている必要がある。デコード後の値（`i64`）だけを
    /// `IntLiteral`に格納してしまうと、`$FF`（255）と`255`（10進）が
    /// 区別できなくなり、意味解析側でdialectチェックのしようがなくなる。
    /// そのため、値のデコード自体は`IntLiteral`と同じ（`i64`をそのまま
    /// 保持する。元の文字列表記や桁数は保持しない）ものの、
    /// 「16進数表記由来である」という事実だけを型レベルで区別して残す。
    HexIntLiteral(i64, Span),
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
    /// 関数呼び出し式（`FUNCTION`の呼び出し。戻り値を持つ式として評価される）。
    ///
    /// 式中に`identifier(args)`の形で現れた場合はパーサーがこのバリアントに
    /// 組み立てる。ただし引数なしの関数呼び出し（`x := Foo`のように
    /// 括弧を省略する伝統的な書き方）は、パーサーの時点では単なる
    /// 変数参照と区別が付かないため`Expr::Identifier`としてパースし、
    /// 意味解析側で「識別子がFUNCTIONシンボルに解決される場合は
    /// 引数なしの呼び出しとして扱う」という形で解決する
    /// （`wasd-sema`のドキュメント参照）。
    FuncCall {
        name: Identifier,
        args: Vec<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::IntLiteral(_, span) => *span,
            Expr::HexIntLiteral(_, span) => *span,
            Expr::RealLiteral(_, span) => *span,
            Expr::StringLiteral(_, span) => *span,
            Expr::BoolLiteral(_, span) => *span,
            Expr::Identifier(ident) => ident.span,
            Expr::BinaryOp { span, .. } => *span,
            Expr::UnaryOp { span, .. } => *span,
            Expr::Paren(_, span) => *span,
            Expr::FuncCall { span, .. } => *span,
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
        assert_eq!(Expr::HexIntLiteral(0xFF, s).span(), s);
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

        let call = Expr::FuncCall {
            name: Identifier::new("Foo", s),
            args: vec![Expr::IntLiteral(1, s)],
            span: s,
        };
        assert_eq!(call.span(), s);
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
