//! WASD Pascalの意味解析。
//!
//! 型検査に加えて、**dialectチェック**をここで行う。パーサーは
//! `Dialect::Iso7185`/`Dialect::Ucsd`のいずれでも同じ文法を受理するため、
//! 「この構文は現在のdialectでは使えない」という判定・報告は本クレートの
//! 責務となる。詳細は`wasd_ast::Dialect`のドキュメントを参照。

pub mod dialect_check;
pub mod typeck;
