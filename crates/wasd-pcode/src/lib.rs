//! p-code (UCSD p-Systemの中間表現) 生成。
//!
//! # 今回のスコープ（最小スコープ）
//!
//! `INTEGER`/`BOOLEAN`型の変数・定数、算術演算（`+ - * DIV MOD`）、比較演算、
//! 論理演算（`AND OR NOT`）、代入文、`IF`/`WHILE`/`REPEAT UNTIL`/`FOR`による
//! 制御構造、`BEGIN...END`の複合文、`PROGRAM ... BEGIN ... END.`全体構造、
//! および`PROGRAM`直下に宣言された`PROCEDURE`/`FUNCTION`の呼び出し
//! （値引数・`VAR`引数・再帰呼び出し・関数の戻り値を含む）を扱う。Step 16
//! からはUCSD拡張の`STRING[n]`型（`PROGRAM`直下のグローバル`VAR`宣言のみ、
//! 文字列リテラルの代入と`WriteLn`への直接渡しのみ）も扱う
//! （[`codegen::CodeGenerator::gen_string_literal_assignment`]・
//! [`builtin::BUILTIN_WRITELN_STRVAR`]参照）。Step 19からは配列型
//! （`PROGRAM`直下のグローバル`VAR`宣言のみ、1次元・`INTEGER`/`BOOLEAN`
//! 要素のみ）の添字アクセス（読み込み・代入）も扱う
//! （[`codegen::CodeGenerator::gen_array_element_address`]参照。新規の
//! p-codeオペコードは追加せず、既存の`LDA`/`IND`/`STI`と算術命令の組み
//! 合わせだけで実現しているため、`pmachine-core`側の改修は不要）。
//! `CASE`、`UNIT`、レコード・ポインタ型、`REAL`/`CHAR`型、`STRING[n]`
//! のローカル変数・仮引数、配列のローカル変数・仮引数・多次元配列・配列
//! 同士の丸ごと代入・実行時の添字範囲チェック、文字列演算（比較・連結・
//! 部分アクセス等）、`WriteLn`以外の組み込み手続き、`PROCEDURE`内
//! `PROCEDURE`のような多段のネストは別ステップに回す
//! （[`codegen::CodeGenerator`]のドキュメント参照）。
//!
//! p-machine命令セットの実バイナリ実行（p-machine本体）は別クレート
//! （`pmachine-core`、未着手）を想定しており、本クレートはそれに向けた
//! IR生成とテキスト表示（逆アセンブリ的なニーモニック表示）までを扱う。
//!
//! # 一次資料への忠実性について
//!
//! [`opcode::UnconfirmedOp`]のドキュメントを参照。オペコード番号・
//! オペランドのバイトエンコーディングは未確認（`Confirmed`/`Unconfirmed`
//! の分離方針に従い、[`opcode::ConfirmedOp`]は意図的に空にしてある）。

pub mod builtin;
pub mod codegen;
pub mod ir;
pub mod opcode;
pub mod text;

pub use builtin::{
    BUILTIN_WRITELN_BOOL, BUILTIN_WRITELN_INT, BUILTIN_WRITELN_NONE, BUILTIN_WRITELN_STRING,
    BUILTIN_WRITELN_STRVAR, KERNEL_SEGMENT,
};
pub use codegen::CodeGenerator;
pub use ir::{Instruction, PCodeModule, RoutineMeta};
pub use opcode::{Address, CodeAddress, ConfirmedOp, Level, Opcode, UnconfirmedOp};
