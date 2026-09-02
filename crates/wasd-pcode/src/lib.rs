//! p-code (UCSD p-Systemの中間表現) 生成。
//!
//! # 今回のスコープ（最小スコープ）
//!
//! `INTEGER`/`BOOLEAN`型の変数・定数、算術演算（`+ - * DIV MOD`）、比較演算、
//! 論理演算（`AND OR NOT`）、代入文、`IF`/`WHILE`/`REPEAT UNTIL`/`FOR`による
//! 制御構造、`BEGIN...END`の複合文、`PROGRAM ... BEGIN ... END.`全体構造、
//! および`PROGRAM`直下に宣言された`PROCEDURE`/`FUNCTION`の呼び出し
//! （値引数・`VAR`引数・再帰呼び出し・関数の戻り値を含む）を扱う。
//! `CASE`、`UNIT`、配列・レコード・ポインタ型、`REAL`/`CHAR`型、組み込み
//! 手続き、`PROCEDURE`内`PROCEDURE`のような多段のネストは別ステップに回す
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
    KERNEL_SEGMENT,
};
pub use codegen::CodeGenerator;
pub use ir::{Instruction, PCodeModule, RoutineMeta};
pub use opcode::{Address, CodeAddress, ConfirmedOp, Level, Opcode, UnconfirmedOp};
