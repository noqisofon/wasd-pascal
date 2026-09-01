//! WASD Pascalのトークナイザ。
//!
//! `wasd-ast`の`Span`/`Diagnostic`を用いる。dialect固有の字句
//! （16進数リテラル `$FF`、コンパイラディレクティブ `(*$I file*)` など）も
//! dialectに関わらず常に字句解析する。dialect違反の判定は行わない
//! （`wasd_ast::Dialect`のドキュメント参照）。

pub mod lexer;
pub mod token;

pub use lexer::Lexer;
pub use token::{Token, TokenKind};
