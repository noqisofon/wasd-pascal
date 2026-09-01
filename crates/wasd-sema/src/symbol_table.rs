//! シンボルテーブル。
//!
//! 今回のスコープでは`PROCEDURE`/`FUNCTION`が未対応のため、スコープの
//! ネストは存在しない。プログラム全体で単一の`SymbolTable`を使う。

use std::collections::HashMap;

use wasd_ast::Span;

use crate::types::Type;

/// シンボルの種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Var,
    Const,
}

/// シンボルテーブルに登録される1エントリ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolInfo {
    pub ty: Type,
    pub kind: SymbolKind,
    pub declared_at: Span,
}

/// `VAR`/`CONST`宣言を名前で引けるようにするテーブル。
///
/// Pascalの識別子はcase-insensitiveなので（`wasd-lexer`が予約語の照合を
/// case-insensitiveに行っているのと同じ理由）、登録・参照のいずれも
/// ASCII小文字化したキーで行う。ただし診断メッセージには（呼び出し側が
/// 保持している）ソース上の元の表記を使うこと。
#[derive(Debug, Default)]
pub struct SymbolTable {
    symbols: HashMap<String, SymbolInfo>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    fn normalize(name: &str) -> String {
        name.to_ascii_lowercase()
    }

    /// シンボルを登録する。同名のシンボルが既に登録されていた場合は
    /// 登録済みの`SymbolInfo`を`Err`で返し、新しい登録は行わない
    /// （呼び出し側が「再宣言」の診断を出すために使う）。
    pub fn declare(&mut self, name: &str, info: SymbolInfo) -> Result<(), SymbolInfo> {
        let key = Self::normalize(name);
        if let Some(existing) = self.symbols.get(&key) {
            return Err(*existing);
        }
        self.symbols.insert(key, info);
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Option<&SymbolInfo> {
        self.symbols.get(&Self::normalize(name))
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
    fn redeclaring_a_name_returns_the_existing_entry() {
        let mut table = SymbolTable::new();
        let first = SymbolInfo {
            ty: Type::Integer,
            kind: SymbolKind::Var,
            declared_at: Span::new(0, 1),
        };
        table.declare("x", first).unwrap();

        let second = SymbolInfo {
            ty: Type::Boolean,
            kind: SymbolKind::Var,
            declared_at: Span::new(10, 11),
        };
        let err = table.declare("x", second).unwrap_err();
        assert_eq!(err, first);
        assert_eq!(table.lookup("x").unwrap().ty, Type::Integer);
    }
}
