//! シンボルテーブル。
//!
//! `PROCEDURE`/`FUNCTION`の導入により、スコープのネストが必須になった。
//! ローカル変数・仮引数は、それを宣言する`PROCEDURE`/`FUNCTION`本体の中でのみ
//! 有効であり、かつ外側（プログラム全体やさらに外側の`PROCEDURE`/`FUNCTION`）の
//! 同名シンボルを覆い隠す（シャドーイングする）。これを表現するため、
//! スコープをスタックとして持ち、`lookup`は内側から外側に向かって
//! 順に探索する。

use std::collections::HashMap;

use wasd_ast::Span;

use crate::types::Type;

/// 仮引数の型と受け渡し方法（値渡し/参照渡し）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSignature {
    pub ty: Type,
    pub by_ref: bool,
}

/// シンボルの種別。
///
/// `Proc`/`Func`は仮引数の型列を保持する（呼び出し側での引数の型・個数の
/// 検査に使う）。`Func`はさらに戻り値の型を持つ。
#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Var,
    Const,
    /// 仮引数。`by_ref`は`VAR`引数（参照渡し）かどうか。
    Param {
        by_ref: bool,
    },
    Proc {
        params: Vec<ParamSignature>,
    },
    Func {
        params: Vec<ParamSignature>,
        return_type: Type,
    },
}

/// シンボルテーブルに登録される1エントリ。
///
/// `ty`の意味はkindによって異なる: `Var`/`Const`/`Param`では宣言された型、
/// `Func`では戻り値の型（`SymbolKind::Func::return_type`と同じ値を複製して
/// 保持しており、`Func`シンボルを通常の識別子と同様に`info.ty`で扱えるように
/// している）。`Proc`には値の型が存在しないため`Type::Error`を仮に入れる
/// （`Proc`シンボルを式として使おうとした場合は`kind`を見て個別にエラーを
/// 出すため、この`ty`の値が直接使われることはない）。
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolInfo {
    pub ty: Type,
    pub kind: SymbolKind,
    pub declared_at: Span,
}

/// `VAR`/`CONST`/`PROCEDURE`/`FUNCTION`宣言・仮引数を名前で引けるようにする
/// テーブル。スコープのスタックを持ち、末尾が最も内側（現在解析中）の
/// スコープを表す。常に少なくとも1つ（プログラム全体のグローバルスコープ）を
/// 持つ。
///
/// Pascalの識別子はcase-insensitiveなので、登録・参照のいずれもASCII
/// 小文字化したキーで行う。ただし診断メッセージには（呼び出し側が
/// 保持している）ソース上の元の表記を使うこと。
#[derive(Debug)]
pub struct SymbolTable {
    scopes: Vec<HashMap<String, SymbolInfo>>,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    fn normalize(name: &str) -> String {
        name.to_ascii_lowercase()
    }

    /// 新しいスコープを push する（`PROCEDURE`/`FUNCTION`本体に入るとき）。
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// 最も内側のスコープを pop する（`PROCEDURE`/`FUNCTION`本体を抜けるとき）。
    /// グローバルスコープ（最初の1つ）は決してpopされない。
    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    /// シンボルを最も内側のスコープに登録する。同名のシンボルが**同じ
    /// スコープに**既に登録されていた場合は登録済みの`SymbolInfo`を`Err`で
    /// 返し、新しい登録は行わない（呼び出し側が「再宣言」の診断を出すために
    /// 使う）。外側のスコープに同名シンボルがあっても、それは
    /// シャドーイングとして正常に許可される（`Err`にはならない）。
    pub fn declare(&mut self, name: &str, info: SymbolInfo) -> Result<(), SymbolInfo> {
        let key = Self::normalize(name);
        let scope = self
            .scopes
            .last_mut()
            .expect("SymbolTable always has at least one (global) scope");
        if let Some(existing) = scope.get(&key) {
            return Err(existing.clone());
        }
        scope.insert(key, info);
        Ok(())
    }

    /// 内側から外側に向かって探索する（レキシカルスコープ）。
    pub fn lookup(&self, name: &str) -> Option<&SymbolInfo> {
        let key = Self::normalize(name);
        self.scopes.iter().rev().find_map(|scope| scope.get(&key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_is_case_insensitive() {
        let mut table = SymbolTable::new();
        table
            .declare(
                "Count",
                SymbolInfo {
                    ty: Type::Integer,
                    kind: SymbolKind::Var,
                    declared_at: Span::new(0, 5),
                },
            )
            .unwrap();

        assert!(table.lookup("count").is_some());
        assert!(table.lookup("COUNT").is_some());
        assert_eq!(table.lookup("count").unwrap().ty, Type::Integer);
    }

    #[test]
    fn redeclaring_a_name_in_the_same_scope_returns_the_existing_entry() {
        let mut table = SymbolTable::new();
        let first = SymbolInfo {
            ty: Type::Integer,
            kind: SymbolKind::Var,
            declared_at: Span::new(0, 1),
        };
        table.declare("x", first.clone()).unwrap();

        let second = SymbolInfo {
            ty: Type::Boolean,
            kind: SymbolKind::Var,
            declared_at: Span::new(10, 11),
        };
        let err = table.declare("x", second).unwrap_err();
        assert_eq!(err, first);
        assert_eq!(table.lookup("x").unwrap().ty, Type::Integer);
    }

    /// 内側のスコープの同名宣言は外側の宣言を隠す（シャドーイング）。
    /// これはエラーにならない。
    #[test]
    fn inner_scope_shadows_outer_scope_without_error() {
        let mut table = SymbolTable::new();
        table
            .declare(
                "x",
                SymbolInfo {
                    ty: Type::Integer,
                    kind: SymbolKind::Var,
                    declared_at: Span::new(0, 1),
                },
            )
            .unwrap();

        table.push_scope();
        table
            .declare(
                "x",
                SymbolInfo {
                    ty: Type::Boolean,
                    kind: SymbolKind::Var,
                    declared_at: Span::new(10, 11),
                },
            )
            .expect("shadowing an outer-scope declaration must not be an error");

        assert_eq!(table.lookup("x").unwrap().ty, Type::Boolean);
        table.pop_scope();
        assert_eq!(table.lookup("x").unwrap().ty, Type::Integer);
    }

    /// `pop_scope`後は内側スコープのシンボルが見えなくなる。
    #[test]
    fn pop_scope_removes_inner_scope_symbols() {
        let mut table = SymbolTable::new();
        table.push_scope();
        table
            .declare(
                "local",
                SymbolInfo {
                    ty: Type::Integer,
                    kind: SymbolKind::Var,
                    declared_at: Span::new(0, 1),
                },
            )
            .unwrap();
        assert!(table.lookup("local").is_some());

        table.pop_scope();
        assert!(table.lookup("local").is_none());
    }

    /// グローバルスコープは`pop_scope`しても消えない。
    #[test]
    fn pop_scope_never_removes_the_global_scope() {
        let mut table = SymbolTable::new();
        table.pop_scope();
        table
            .declare(
                "x",
                SymbolInfo {
                    ty: Type::Integer,
                    kind: SymbolKind::Var,
                    declared_at: Span::new(0, 1),
                },
            )
            .unwrap();
        assert!(table.lookup("x").is_some());
    }
}
