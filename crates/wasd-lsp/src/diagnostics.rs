//! `wasd_driver::Diagnostic`からLSPの`textDocument/publishDiagnostics`が
//! 期待する`Diagnostic`型への変換。

use tower_lsp::lsp_types::{DiagnosticSeverity, NumberOrString};
use wasd_driver::Severity;

use crate::position::span_to_range;

/// このサーバーが発行する診断の`source`フィールドに使う値。
const DIAGNOSTIC_SOURCE: &str = "wasd-pascal";

fn to_lsp_severity(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
        Severity::Hint => DiagnosticSeverity::HINT,
    }
}

/// `wasd-driver`の[`wasd_driver::Diagnostic`]を、`source`（診断が発生した
/// ドキュメントの全文）を使って位置情報を解決したLSPの`Diagnostic`へ変換する。
pub fn to_lsp_diagnostic(
    diag: &wasd_driver::Diagnostic,
    source: &str,
) -> tower_lsp::lsp_types::Diagnostic {
    tower_lsp::lsp_types::Diagnostic {
        range: span_to_range(source, diag.span),
        severity: Some(to_lsp_severity(diag.severity)),
        code: diag.code.clone().map(NumberOrString::String),
        code_description: None,
        source: Some(DIAGNOSTIC_SOURCE.to_string()),
        message: diag.message.clone(),
        related_information: None,
        tags: None,
        data: None,
    }
}

/// `wasd-driver::compile`が返す診断の列をまとめてLSPの`Diagnostic`へ変換する。
pub fn to_lsp_diagnostics(
    diagnostics: &[wasd_driver::Diagnostic],
    source: &str,
) -> Vec<tower_lsp::lsp_types::Diagnostic> {
    diagnostics
        .iter()
        .map(|diag| to_lsp_diagnostic(diag, source))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;
    use wasd_driver::{compile, CompileOptions, Diagnostic, Span};

    #[test]
    fn converts_severity_and_message() {
        let source = "PROGRAM Foo;\nBEGIN\nEND.\n";
        let diag = Diagnostic::new(Span::new(8, 11), Severity::Error, "something broke");

        let lsp_diag = to_lsp_diagnostic(&diag, source);

        assert_eq!(lsp_diag.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(lsp_diag.message, "something broke");
        assert_eq!(lsp_diag.source.as_deref(), Some(DIAGNOSTIC_SOURCE));
        assert_eq!(lsp_diag.range.start, Position::new(0, 8));
        assert_eq!(lsp_diag.range.end, Position::new(0, 11));
    }

    #[test]
    fn converts_code_when_present() {
        let source = "PROGRAM Foo;\nEND.\n";
        let diag = Diagnostic::new(Span::new(0, 1), Severity::Warning, "note").with_code("E0042");

        let lsp_diag = to_lsp_diagnostic(&diag, source);

        assert_eq!(
            lsp_diag.code,
            Some(NumberOrString::String("E0042".to_string()))
        );
        assert_eq!(lsp_diag.severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn a_valid_program_produces_no_diagnostics() {
        let source = r#"
            PROGRAM Hello;
            VAR
                answer: INTEGER;
            BEGIN
                answer := 42
            END.
        "#;

        let result = compile(source, &CompileOptions::default());
        let lsp_diags = to_lsp_diagnostics(&result.diagnostics, source);

        assert!(
            lsp_diags.is_empty(),
            "expected no diagnostics, got {lsp_diags:?}"
        );
    }

    #[test]
    fn a_type_error_produces_an_error_diagnostic_at_the_right_location() {
        let source = r#"
            PROGRAM TypeError;
            VAR
                flag: BOOLEAN;
                count: INTEGER;
            BEGIN
                flag := TRUE;
                count := flag + 1
            END.
        "#;

        let result = compile(source, &CompileOptions::default());
        let lsp_diags = to_lsp_diagnostics(&result.diagnostics, source);

        assert_eq!(lsp_diags.len(), 1, "diagnostics: {lsp_diags:?}");
        assert_eq!(lsp_diags[0].severity, Some(DiagnosticSeverity::ERROR));
        assert!(lsp_diags[0].message.contains("BOOLEAN"));
        // The offending line is where `count := flag + 1` appears.
        let expected_line = source
            .lines()
            .position(|line| line.contains("count := flag + 1"))
            .unwrap() as u32;
        assert_eq!(lsp_diags[0].range.start.line, expected_line);
    }
}
