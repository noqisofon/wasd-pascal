//! 文のASTノード。
//!
//! 今回のスコープ: 代入・IF・WHILE・複合文・引数なし/単純な式引数のみの
//! 手続き呼び出し。`FOR`/`REPEAT UNTIL`/`CASE`は含めない。

use crate::expr::Expr;
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
/// `#[non_exhaustive]`: 将来`FOR`/`REPEAT UNTIL`/`CASE`などのバリアントを
/// 追加してもワークスペース内の他クレートでの`match`が静かに壊れない
/// （ワイルドカードアームがなければコンパイルエラーになり、対応漏れに気付ける）
/// ようにするため。
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
    Compound(Block),
    /// 手続き呼び出し文。今回のスコープでは引数は単純な式のみ
    /// （`var`引数や既定引数のような特殊な引数渡しは扱わない）。
    ProcCall {
        name: Identifier,
        args: Vec<Expr>,
        span: Span,
    },
}

impl Statement {
    pub fn span(&self) -> Span {
        match self {
            Statement::Assignment { span, .. } => *span,
            Statement::If { span, .. } => *span,
            Statement::While { span, .. } => *span,
            Statement::Compound(block) => block.span,
            Statement::ProcCall { span, .. } => *span,
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
