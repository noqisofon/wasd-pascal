//! 型検査で使う内部型表現。
//!
//! `wasd_ast::TypeExpr`はソース上に書かれた型注釈をそのまま表す構文要素
//! （`Span`を持ち、診断のために「どこに書かれた型か」を保持する）だが、
//! 型検査中に式やシンボルへ付与する型はソース位置を持たない値でよいため、
//! ここで区別して`Type`として定義する。

use std::fmt;

/// 型検査が扱う型。
///
/// 組み込み型（INTEGER/REAL/BOOLEAN/CHAR）に加え、UCSD拡張の`STRING[n]`型
/// （`wasd_ast::TypeExpr::StringN`）を扱う。配列・レコード型などは今後の拡張。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Integer,
    Real,
    Boolean,
    Char,
    /// UCSD拡張: `STRING[n]`（`n`は最大長）。
    ///
    /// # 既知の制限: 長さの異なる`STRING[n]`同士の代入互換性は判定しない
    ///
    /// 実際のUCSD Pascalでは（一次資料未確認だが慣用的に）異なる最大長を
    /// 持つ`STRING`変数同士でも、実際の文字列長が収まる限り代入可能である
    /// ことが多いと理解している。今回のスコープではその互換性判定までは
    /// 実装せず、`Type::StringN(n)`同士は`n`が完全に一致する場合のみ
    /// （`#[derive(PartialEq)]`による構造的等価性）互換とみなす暫定実装。
    StringN(usize),
    /// 型検査に失敗した式・宣言に割り当てるプレースホルダ型。
    ///
    /// `Type::Error`同士、または`Type::Error`と他の型との間の演算・比較・
    /// 代入では追加の診断を出さない。これにより、1つの型エラーが後続の
    /// 無関係な型エラーを連鎖的に誘発する（カスケードエラー）のを防ぐ。
    Error,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Integer => f.write_str("INTEGER"),
            Type::Real => f.write_str("REAL"),
            Type::Boolean => f.write_str("BOOLEAN"),
            Type::Char => f.write_str("CHAR"),
            Type::StringN(n) => write!(f, "STRING[{n}]"),
            Type::Error => f.write_str("<error>"),
        }
    }
}
