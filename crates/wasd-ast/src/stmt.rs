//! 文のASTノード。
//!
//! 今回のスコープ: 代入・IF・WHILE・FOR・REPEAT UNTIL・CASE・複合文・
//! 引数なし/単純な式引数のみの手続き呼び出し。

use crate::expr::{Expr, Literal};
use crate::ident::Identifier;
use crate::span::Span;

/// `BEGIN ... END`で束ねられた文の並び。
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub span: Span,
}

/// 文。
///
/// `#[non_exhaustive]`: 将来のバリアント追加（配列型導入後の`WITH`文など）で
/// もワークスペース内の他クレートでの`match`が静かに壊れない
/// （ワイルドカードアームがなければコンパイルエラーになり、対応漏れに
/// 気付ける）ようにするため。
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Statement {
    Assignment {
        target: Identifier,
        value: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then_branch: Box<Statement>,
        else_branch: Option<Box<Statement>>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Box<Statement>,
        span: Span,
    },
    /// `FOR var := start (TO|DOWNTO) end DO body`。
    For {
        var: Identifier,
        start: Expr,
        end: Expr,
        direction: ForDirection,
        body: Box<Statement>,
        span: Span,
    },
    /// `REPEAT stmt1; stmt2; ... UNTIL cond`。
    ///
    /// `WHILE`とは異なり`BEGIN...END`なしで複数文を直接書ける
    /// （`REPEAT`〜`UNTIL`自体が文の並びの区切りとなるため）。また、
    /// 条件を末尾で判定するため、本体`body`は`WHILE`とは違って
    /// 少なくとも1回は実行される（`Vec<Statement>`として直接保持している
    /// ことと、条件`until_cond`が本体の"後"に置かれているというこの
    /// フィールドの並び自体が、その意味論の違いを表している）。
    Repeat {
        body: Vec<Statement>,
        until_cond: Expr,
        span: Span,
    },
    /// `CASE selector OF label1, label2: stmt1; label3: stmt2; ... [OTHERWISE stmtN] END`。
    ///
    /// `otherwise`はUCSD拡張の`OTHERWISE`句（どの`label`にも一致しない場合の
    /// デフォルト分岐）。`Iso7185`では使用不可であり、dialectチェックは
    /// `wasd-sema`が行う（`wasd_ast::Dialect`のドキュメント参照）。パーサーは
    /// dialectに関わらず常に受理する。
    Case {
        selector: Expr,
        branches: Vec<CaseBranch>,
        otherwise: Option<Box<Statement>>,
        span: Span,
    },
    Compound(Block),
    /// 手続き呼び出し文。今回のスコープでは引数は単純な式のみ
    /// （`var`引数や既定引数のような特殊な引数渡しは扱わない）。
    ProcCall {
        name: Identifier,
        args: Vec<Expr>,
        span: Span,
    },
    /// UCSD拡張: コンパイラディレクティブ `(*$I foo.pas*)`。
    ///
    /// 今回のスコープでは実際のプリプロセッサ的な動作（ファイルの
    /// インクルードなど）は実装せず、ディレクティブの存在を認識し
    /// dialectチェックの対象とするところまでに留める
    /// （`wasd-sema`のドキュメント参照）。文の並びの中に現れた場合のみを
    /// 扱い、宣言部など他の位置に現れた場合は今回のスコープ外
    /// （既存の`skip_unsupported_section`等の読み飛ばし経路に委ねる）。
    CompilerDirective {
        name: String,
        args: String,
        span: Span,
    },
}

/// `CASE`文中の1分岐（`label1, label2: statement`の部分）。
///
/// `body`は`Box`で包まない。`CaseBranch`自体が常に`Vec<CaseBranch>`
/// （ヒープ上）の要素として存在するため、`Statement`を直接持たせても
/// 無限サイズにはならない。
#[derive(Debug, Clone, PartialEq)]
pub struct CaseBranch {
    pub labels: Vec<Literal>,
    pub body: Statement,
    pub span: Span,
}

/// `FOR`文のループ方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForDirection {
    /// `FOR i := 1 TO 10`: ループ変数を1ずつ増やす。
    To,
    /// `FOR i := 10 DOWNTO 1`: ループ変数を1ずつ減らす。
    DownTo,
}

impl Statement {
    pub fn span(&self) -> Span {
        match self {
            Statement::Assignment { span, .. } => *span,
            Statement::If { span, .. } => *span,
            Statement::While { span, .. } => *span,
            Statement::For { span, .. } => *span,
            Statement::Repeat { span, .. } => *span,
            Statement::Case { span, .. } => *span,
            Statement::Compound(block) => block.span,
            Statement::ProcCall { span, .. } => *span,
            Statement::CompilerDirective { span, .. } => *span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(name: &str, s: Span) -> Identifier {
        Identifier::new(name, s)
    }

    /// ネストした`IF`/`WHILE`のStatementツリーを構築できること。
    #[test]
    fn builds_nested_if_and_while_statement_tree() {
        let s = Span::new(0, 1);

        let inner_assign = Statement::Assignment {
            target: ident("x", s),
            value: Expr::IntLiteral(1, s),
            span: s,
        };

        let while_stmt = Statement::While {
            cond: Expr::Identifier(ident("running", s)),
            body: Box::new(inner_assign.clone()),
            span: s,
        };

        let if_stmt = Statement::If {
            cond: Expr::Identifier(ident("cond", s)),
            then_branch: Box::new(while_stmt.clone()),
            else_branch: Some(Box::new(inner_assign.clone())),
            span: s,
        };

        match &if_stmt {
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                assert_eq!(**then_branch, while_stmt);
                assert_eq!(else_branch.as_deref(), Some(&inner_assign));
            }
            _ => panic!("expected Statement::If"),
        }
        assert_eq!(if_stmt.span(), s);
    }

    #[test]
    fn compound_span_comes_from_block() {
        let block_span = Span::new(0, 10);
        let block = Block {
            statements: vec![],
            span: block_span,
        };
        assert_eq!(Statement::Compound(block).span(), block_span);
    }
}
