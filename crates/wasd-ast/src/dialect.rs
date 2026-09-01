//! Pascalの方言(dialect)を表す型。

/// Pascalの方言。デフォルトはISO 7185準拠の標準Pascal。
/// UCSD拡張はオプトインで有効化する（gccの `-std=` に相当する発想）。
///
/// # 設計方針
///
/// パーサーはdialectに関わらず単一である。UCSD拡張構文
/// （`UNIT`/`INTERFACE`/`IMPLEMENTATION`、`STRING[n]`、`OTHERWISE`、
/// 16進数リテラル `$FF`、コンパイラディレクティブ `(*$I file*)` など）も、
/// 文法上は常にパース可能とする。パーサーレベルではdialect違反を拒否しない。
///
/// dialectのチェック（「この構文は現在のdialectでは使えない」というエラー）は
/// **意味解析フェーズ（`wasd-sema`）で行う**。
///
/// 理由:
/// - パーサーを1本に保てる（dialectごとの文法分岐や二重実装を避けられる）。
/// - 構文エラーではなく意味エラーとして報告できるため、メッセージがわかりやすい。
/// - LSPでの診断（`textDocument/publishDiagnostics`）に載せやすい。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    #[default]
    Iso7185,
    Ucsd,
}
