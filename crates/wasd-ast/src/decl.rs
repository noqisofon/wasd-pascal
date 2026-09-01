//! 宣言（`PROGRAM`/`VAR`/`CONST`など）のASTノード。
//!
//! 今回のスコープ: `PROGRAM`ヘッダ、単一の`BEGIN...END.`ブロック、
//! `VAR`セクション（組み込み型のみ）、`CONST`セクション（リテラル値のみ）。
//! `UNIT`/`INTERFACE`/`IMPLEMENTATION`、`PROCEDURE`/`FUNCTION`、`TYPE`セクション
//! （配列・レコード・ポインタ型を含む）は今回は含めない。
//!
//! `Program`は将来`UNIT`宣言と並ぶ「コンパイル単位」の一種として
//! `enum CompilationUnit { Program(Program), Unit(UnitDecl) }`のような形で
//! 包まれる可能性がある。そのため`Program`自体は独立した`struct`のままにしておき、
//! 今回はそのラッパーenumを先回りして作らない。

use crate::expr::Literal;
use crate::ident::Identifier;
use crate::span::Span;
use crate::stmt::Block;

/// `PROGRAM <identifier>; ... BEGIN ... END.` 全体。
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub name: Identifier,
    pub const_decls: Vec<ConstDecl>,
    pub var_decls: Vec<VarDecl>,
    pub body: Block,
    pub span: Span,
}

/// `VAR`セクション中の1グループ（`x, y: Integer;`のような識別子リストと型の組）。
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub names: Vec<Identifier>,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `CONST`セクション中の1宣言（`Pi = 3.14;`のような識別子とリテラル値の組）。
#[derive(Debug, Clone, PartialEq)]
pub struct ConstDecl {
    pub name: Identifier,
    pub value: Literal,
    pub span: Span,
}

/// 型。今回は組み込み型のみ。
///
/// `Expr`のリテラルバリアントと同様、型を書いたソース上の位置を
/// バリアントごとに`Span`として保持する（型名の綴りに対する診断のため）。
///
/// `#[non_exhaustive]`: 将来`Array`/`Record`/`Pointer`/`StringN`などを
/// 追加する際に、既存の`match`をワイルドカードなしで壊さないようにするため。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TypeExpr {
    Integer(Span),
    Real(Span),
    Boolean(Span),
    Char(Span),
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Integer(span)
            | TypeExpr::Real(span)
            | TypeExpr::Boolean(span)
            | TypeExpr::Char(span) => *span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::Expr;
    use crate::stmt::Statement;

    fn ident(name: &str, s: Span) -> Identifier {
        Identifier::new(name, s)
    }

    /// `PROGRAM Foo; BEGIN END.`に相当する最小ASTを手で構築し、
    /// フィールドアクセスが期待通り動くこと。
    #[test]
    fn builds_minimal_program() {
        let program_span = Span::new(0, 21);
        let name_span = Span::new(8, 11);
        let body_span = Span::new(13, 21);

        let program = Program {
            name: ident("Foo", name_span),
            const_decls: vec![],
            var_decls: vec![],
            body: Block {
                statements: vec![],
                span: body_span,
            },
            span: program_span,
        };

        assert_eq!(program.name.name, "Foo");
        assert_eq!(program.name.span, name_span);
        assert!(program.const_decls.is_empty());
        assert!(program.var_decls.is_empty());
        assert!(program.body.statements.is_empty());
        assert_eq!(program.span, program_span);
    }

    /// `VarDecl`/`ConstDecl`を含むASTを構築し、型が正しく組めること。
    #[test]
    fn builds_program_with_var_and_const_decls() {
        let s = Span::new(0, 1);

        let var_decl = VarDecl {
            names: vec![ident("x", s), ident("y", s)],
            ty: TypeExpr::Integer(s),
            span: s,
        };
        let const_decl = ConstDecl {
            name: ident("MaxScore", s),
            value: Literal::Real(3.5, s),
            span: s,
        };

        let program = Program {
            name: ident("Foo", s),
            const_decls: vec![const_decl.clone()],
            var_decls: vec![var_decl.clone()],
            body: Block {
                statements: vec![],
                span: s,
            },
            span: s,
        };

        assert_eq!(program.var_decls[0].names.len(), 2);
        assert_eq!(program.var_decls[0].ty, TypeExpr::Integer(s));
        assert_eq!(program.const_decls[0].value, Literal::Real(3.5, s));
        assert_eq!(var_decl.ty.span(), s);
        assert_eq!(const_decl.value.span(), s);
    }

    /// 各ノードが`Span`を保持していること（代表的なノードで確認）。
    #[test]
    fn nodes_carry_spans() {
        let decl_span = Span::new(5, 20);
        let var_decl = VarDecl {
            names: vec![Identifier::new("x", Span::new(5, 6))],
            ty: TypeExpr::Boolean(Span::new(8, 15)),
            span: decl_span,
        };
        assert_eq!(var_decl.span, decl_span);
        assert_eq!(var_decl.names[0].span, Span::new(5, 6));
        assert_eq!(var_decl.ty.span(), Span::new(8, 15));

        let stmt = Statement::ProcCall {
            name: Identifier::new("WriteLn", Span::new(0, 7)),
            args: vec![Expr::Identifier(Identifier::new("x", Span::new(8, 9)))],
            span: Span::new(0, 10),
        };
        assert_eq!(stmt.span(), Span::new(0, 10));
    }
}
