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
/// （`wasd_ast::TypeExpr::StringN`）、配列型・レコード型・ポインタ型、
/// および`NIL`リテラルの型を扱う。
///
/// # 設計判断: 配列は構造的型付け、レコードは名前的型付け
///
/// タスク文書の指示通り、配列は「同じ次元・要素型・添字範囲を持つ配列
/// 同士は同じ型とみなす」という構造的型付けのルールを採る
/// （[`ArrayType`]の`#[derive(PartialEq)]`がそのままこのルールになる）。
///
/// 一方レコードは「同じ型宣言に由来するレコードのみ代入可能」という
/// 名前的型付けに近いルールを採る。ISO 7185自体はレコード型の型同一性を
/// 構造的に定義していない（実装依存の余地がある）ため、この実装判断は
/// タスク文書の指示に従い簡略化したもの。`Type::Record`は完全な
/// フィールド構造ではなく、識別用の名前（`TYPE`宣言名、または`VAR`/
/// 仮引数/フィールドの型注釈に直接書かれた無名`RECORD`の場合は
/// `SemaContext`が生成する合成名）のみを保持する軽量なハンドルであり、
/// 実際のフィールド一覧は`SemaContext`が別途保持するレコードレジストリ
/// （`crate::typeck::RecordInfo`）で名前をキーに引く。この間接化は
/// 再帰的なレコード定義（`Node = RECORD next: ^Node END`のような
/// 自己参照）を、`Type`自体を無限サイズにすることなく表現するためにも
/// 必要（ポインタが指す先はこの軽量な名前ハンドルであり、フィールド
/// 一覧を丸ごと埋め込むわけではないため、循環しても問題にならない）。
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// 配列型。`ArrayType`のドキュメント参照。
    Array(Box<ArrayType>),
    /// レコード型。中身は`SemaContext`のレコードレジストリで名前を
    /// キーに引く（このモジュールのドキュメント参照）。
    Record(String),
    /// ポインタ型 `^T`。
    Pointer(Box<Type>),
    /// `NIL`リテラルの型。任意の`Type::Pointer(_)`と比較・代入互換になる
    /// 特別な型（`crate::typeck`の代入互換性判定・`=`/`<>`の型検査を参照）。
    Nil,
    /// 型検査に失敗した式・宣言に割り当てるプレースホルダ型。
    ///
    /// `Type::Error`同士、または`Type::Error`と他の型との間の演算・比較・
    /// 代入では追加の診断を出さない。これにより、1つの型エラーが後続の
    /// 無関係な型エラーを連鎖的に誘発する（カスケードエラー）のを防ぐ。
    Error,
}

/// 配列型の中身。`低添字..高添字`の範囲と要素型を持つ。多次元配列は
/// `element`が入れ子の`Type::Array`になる形で表現する
/// （`wasd_ast::decl::TypeExpr::Array`のドキュメント参照）。
///
/// `#[derive(PartialEq)]`がそのまま「同じ次元・要素型・添字範囲を持つ
/// 配列同士は同じ型とみなす」という構造的型付けのルールになる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayType {
    pub low: i64,
    pub high: i64,
    pub element: Type,
    pub packed: bool,
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Integer => f.write_str("INTEGER"),
            Type::Real => f.write_str("REAL"),
            Type::Boolean => f.write_str("BOOLEAN"),
            Type::Char => f.write_str("CHAR"),
            Type::StringN(n) => write!(f, "STRING[{n}]"),
            Type::Array(arr) => {
                if arr.packed {
                    write!(f, "PACKED ")?;
                }
                write!(f, "ARRAY[{}..{}] OF {}", arr.low, arr.high, arr.element)
            }
            Type::Record(name) => {
                if is_anonymous_record_name(name) {
                    f.write_str("<anonymous record>")
                } else {
                    write!(f, "RECORD {name}")
                }
            }
            Type::Pointer(pointee) => write!(f, "^{pointee}"),
            Type::Nil => f.write_str("NIL"),
            Type::Error => f.write_str("<error>"),
        }
    }
}

/// `SemaContext`が無名`RECORD`型（`TYPE`宣言を経ない、`VAR`/仮引数/
/// フィールドの型注釈に直接書かれた`RECORD ... END`）に割り当てる合成名の
/// 接頭辞。診断メッセージ表示時にこの接頭辞を検出して人間が読める形に
/// 差し替える（`Display`実装参照）。
pub(crate) const ANONYMOUS_RECORD_PREFIX: &str = "anon#";

pub(crate) fn is_anonymous_record_name(name: &str) -> bool {
    name.starts_with(ANONYMOUS_RECORD_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrays_are_structurally_compared() {
        let a = Type::Array(Box::new(ArrayType {
            low: 1,
            high: 10,
            element: Type::Integer,
            packed: false,
        }));
        let b = Type::Array(Box::new(ArrayType {
            low: 1,
            high: 10,
            element: Type::Integer,
            packed: false,
        }));
        let c = Type::Array(Box::new(ArrayType {
            low: 1,
            high: 11,
            element: Type::Integer,
            packed: false,
        }));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn records_are_compared_by_name_only() {
        assert_eq!(Type::Record("Node".into()), Type::Record("Node".into()));
        assert_ne!(Type::Record("Node".into()), Type::Record("Other".into()));
    }

    #[test]
    fn pointer_display_shows_pointee() {
        let ty = Type::Pointer(Box::new(Type::Record("Node".into())));
        assert_eq!(ty.to_string(), "^RECORD Node");
    }
}
