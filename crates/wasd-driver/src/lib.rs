//! `wasdc`（CLI）とLSPサーバーが共有するコアAPI。
//!
//! ソース読み込みからdialect選択、字句/構文/意味解析パイプラインの
//! 呼び出しまでをここに集約し、CLIとLSPの双方が同じロジックを再利用する。
//! [`run`]はさらにp-code生成の先、`pmachine-core`による実行までを一気通貫
//! で行う（`wasdc run`向け）。

pub mod compile;
pub mod pcode;
pub mod run;
pub mod session;
pub mod source_loc;

pub use compile::{compile, CompileOptions, CompileResult};
pub use pcode::{compile_to_pcode, PCodeEmitResult};
pub use pmachine_core::RuntimeError;
pub use run::{run as run_program, RunResult};
pub use source_loc::{locate, LineCol, SourceLocation};
pub use wasd_ast::{Diagnostic, Dialect, Severity, Span};
pub use wasd_pcode::PCodeModule;
