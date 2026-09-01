//! dialectチェック。
//!
//! パーサーはdialectに関わらずUCSD拡張構文（`UNIT`/`INTERFACE`/
//! `IMPLEMENTATION`/`USES`、`STRING[n]`、`CASE`文の`OTHERWISE`句、
//! `$FF`のような16進数リテラル、`(*$I file*)`のようなコンパイラ
//! ディレクティブ等）を受理する。それらが現在のdialectで許可されているか
//! どうかを判定し、`wasd_ast::Diagnostic`として報告するのが本モジュールの
//! 役割。
//!
//! 実際の呼び出しは[`crate::typeck::SemaContext`]の各`check_*`メソッドが、
//! ASTを走査しながら該当するUCSD拡張構文に遭遇するたびに行う
//! （`SemaContext::check_dialect_gate`はここの[`check_dialect_gate`]を
//! 薄くラップしたもの）。

use wasd_ast::{Dialect, Diagnostic, Severity, Span};

/// UCSD拡張構文が`dialect`のもとで許可されているかを判定する。
///
/// `dialect != required`であれば、その構文がUCSD dialectを要求する旨の
/// `Diagnostic`を返す。呼び出し元はこれを蓄積すること。許可されている
/// 場合は`None`を返す。
///
/// エラー発生後もASTの残りの意味解析を継続できるよう、この関数自体は
/// 単に診断を生成するだけで、呼び出し元の走査を止めるような副作用は
/// 一切持たない（呼び出し元が診断を積んだ後もそのまま処理を続けられる
/// ようにするのが目的）。
pub fn check_dialect_gate(
    dialect: Dialect,
    span: Span,
    feature: &str,
    required: Dialect,
) -> Option<Diagnostic> {
    if dialect == required {
        return None;
    }
    let requirement = match required {
        Dialect::Ucsd => "requires UCSD dialect (use --std=ucsd)",
        Dialect::Iso7185 => "requires ISO 7185 dialect (use --std=iso7185)",
    };
    Some(Diagnostic::new(
        span,
        Severity::Error,
        format!("{feature} {requirement}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_when_dialect_matches_requirement() {
        let span = Span::new(0, 4);
        assert!(check_dialect_gate(Dialect::Ucsd, span, "UNIT declarations", Dialect::Ucsd).is_none());
    }

    #[test]
    fn returns_an_error_diagnostic_when_dialect_does_not_match() {
        let span = Span::new(0, 4);
        let diag = check_dialect_gate(Dialect::Iso7185, span, "UNIT declarations", Dialect::Ucsd)
            .expect("expected a diagnostic");
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.span, span);
        assert!(diag.message.contains("UNIT declarations"));
        assert!(diag.message.contains("UCSD"));
    }
}
