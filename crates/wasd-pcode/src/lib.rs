//! p-code (UCSD p-Systemの中間表現) 生成。
//!
//! # 今回のスコープ（最小スコープ）
//!
//! `INTEGER`/`BOOLEAN`型の変数・定数、算術演算（`+ - * DIV MOD`）、比較演算、
//! 論理演算（`AND OR NOT`）、代入文、`IF`/`WHILE`/`REPEAT UNTIL`/`FOR`による
//! 制御構造、`BEGIN...END`の複合文、最小限の`PROGRAM ... BEGIN ... END.`
//! 全体構造のみを扱う。`PROCEDURE`/`FUNCTION`、`CASE`、`UNIT`、配列・
//! レコード・ポインタ型、`REAL`/`CHAR`型、組み込み手続きは別ステップに回す
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

pub mod codegen;
pub mod ir;
pub mod opcode;
pub mod text;

pub use codegen::CodeGenerator;
pub use ir::{Instruction, PCodeModule};
pub use opcode::{Address, CodeAddress, ConfirmedOp, Opcode, UnconfirmedOp};
