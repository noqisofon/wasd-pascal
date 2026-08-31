//! AST定義、ソース位置(`Span`)、`Dialect`、`Diagnostic`を提供する。
//!
//! このクレートはワークスペース内の他のどのクレートにも依存しない。
//! 依存されるだけの最下層のクレートとして扱うこと。

pub mod decl;
pub mod diagnostic;
pub mod dialect;
pub mod expr;
pub mod span;
pub mod stmt;

pub use diagnostic::{Diagnostic, Severity};
pub use dialect::Dialect;
pub use span::Span;
