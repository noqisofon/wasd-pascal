//! AST定義、ソース位置(`Span`)、`Dialect`、`Diagnostic`を提供する。
//!
//! このクレートはワークスペース内の他のどのクレートにも依存しない。
//! 依存されるだけの最下層のクレートとして扱うこと。

pub mod decl;
pub mod diagnostic;
pub mod dialect;
pub mod expr;
pub mod ident;
pub mod span;
pub mod stmt;

pub use decl::{ConstDecl, FuncDecl, ParamDecl, ProcDecl, Program, TypeExpr, VarDecl};
pub use diagnostic::{Diagnostic, Severity};
pub use dialect::Dialect;
pub use expr::{BinOp, Expr, Literal, UnOp};
pub use ident::Identifier;
pub use span::Span;
pub use stmt::{Block, CaseBranch, ForDirection, Statement};
