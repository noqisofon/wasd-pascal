//! 型検査で使う内部型表現。
//!
//! `wasd_ast::TypeExpr`はソース上に書かれた型注釈をそのまま表す構文要素
//! （`Span`を持ち、診断のために「どこに書かれた型か」を保持する）だが、
//! 型検査中に式やシンボルへ付与する型はソース位置を持たない値でよいため、
//! ここで区別して`Type`として定義する。

use std::fmt;

/// 型検査が扱う型。
///
/// 今回のスコープでは`wasd_ast::TypeExpr`が表せる組み込み型
/// （INTEGER/REAL/BOOLEAN/CHAR）のみを扱う。配列・レコード・`STRING[n]`型
/// などは今後の拡張。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Integer,
    Real,
    Boolean,
    Char,
    /// 型検査に失敗した式・宣言に割り当てるプレースホルダ型。
    ///
    /// `Type::Error`同士、または`Type::Error`と他の型との間の演算・比較・
    /// 代入では追加の診断を出さない。これにより、1つの型エラーが後続の
    /// 無関係な型エラーを連鎖的に誘発する（カスケードエラー）のを防ぐ。
    Error,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Type::Integer => "INTEGER",
            Type::Real => "REAL",
            Type::Boolean => "BOOLEAN",
            Type::Char => "CHAR",
            Type::Error => "<error>",
        };
        f.write_str(s)
    }
}
