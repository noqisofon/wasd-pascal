//! CLIとLSPの両方から使う共通の診断型。
//!
//! `Diagnostic`は将来的にLSPの`textDocument/publishDiagnostics`が期待する
//! `Diagnostic`構造（range/severity/message/code）へそのまま変換される想定であり、
//! フィールド構成もそれを意識している。CLI側はこれをテキスト整形して出力する。

use crate::span::Span;

/// 診断の重大度。LSPの`DiagnosticSeverity`と1対1で対応する想定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

/// レキサ・パーサー・意味解析のいずれかから発生する診断。
///
/// # dialect違反の例
///
/// dialectチェックは`wasd-sema`が行う（`Dialect`のドキュメント参照）。
/// 例えば`Dialect::Iso7185`のもとで`UNIT`宣言を検出した場合、
/// 次のような診断を発する想定（今回は未実装、方針のみ）:
///
/// ```text
/// UNIT declarations require UCSD dialect (use --std=ucsd)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub message: String,
    /// 将来的なエラーコード用（例: "E0042"）。
    pub code: Option<String>,
}

impl Diagnostic {
    pub fn new(span: Span, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            span,
            severity,
            message: message.into(),
            code: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}
