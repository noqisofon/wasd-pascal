//! dialectチェック（プレースホルダ）。
//!
//! パーサーはdialectに関わらずUCSD拡張構文（`UNIT`/`INTERFACE`/
//! `IMPLEMENTATION`、`STRING[n]`、`OTHERWISE`、`$FF`のような16進数リテラル、
//! `(*$I file*)`のようなコンパイラディレクティブ等）を受理する。
//! それらが現在のdialectで許可されているかどうかを判定し、
//! `wasd_ast::Diagnostic`として報告するのが本モジュールの役割。
//!
//! 実装はAST定義が固まってから行う。以下はイメージ（今回は未実装）:
//!
//! ```ignore
//! use wasd_ast::{Dialect, Diagnostic, Severity, Span};
//!
//! fn check_unit_decl(dialect: Dialect, span: Span) -> Option<Diagnostic> {
//!     if dialect != Dialect::Ucsd {
//!         return Some(
//!             Diagnostic::new(
//!                 span,
//!                 Severity::Error,
//!                 "UNIT declarations require UCSD dialect (use --std=ucsd)",
//!             )
//!             .with_code("E0001"),
//!         );
//!     }
//!     None
//! }
//! ```
