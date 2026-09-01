//! WASD Pascalのパーサー。
//!
//! パーサーはdialectに関わらず単一である。UCSD拡張構文
//! （`UNIT`/`INTERFACE`/`IMPLEMENTATION`、`STRING[n]`、`OTHERWISE`、
//! 16進数リテラル `$FF`、コンパイラディレクティブなど）も文法上は常に
//! パース可能とし、パーサーレベルではdialect違反を拒否しない。
//!
//! dialectチェックは`wasd-sema`が担当する。詳細は`wasd_ast::Dialect`の
//! ドキュメントを参照。

pub mod parser;
