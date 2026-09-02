//! p-code命令のIR表現。
//!
//! オペコード自体の定義（`Confirmed`/`Unconfirmed`の分離）は
//! [`crate::opcode`]を参照。このモジュールは命令列（[`PCodeModule`]）と
//! 個々の命令（[`Instruction`]）を扱う。

use wasd_ast::Span;

use crate::opcode::Opcode;

/// p-code命令列中の1命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub opcode: Opcode,
    /// この命令の生成元となったASTノードのソース位置（診断・デバッグ用）。
    pub span: Span,
}

/// 1つの`PROGRAM`から生成されたp-code命令列。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PCodeModule {
    pub instructions: Vec<Instruction>,
    /// グローバルデータ領域が必要とするワード数（ユーザー宣言の`VAR`に
    /// 加え、コード生成器が導入した一時変数を含む）。
    pub global_data_words: u16,
}
