//! 識別子。
//!
//! `decl`/`stmt`/`expr`のいずれからも参照される共通ノードのため、
//! 独立したモジュールに置く。

use crate::span::Span;

/// ソース上の識別子（変数名・定数名・プログラム名など）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identifier {
    pub name: String,
    pub span: Span,
}

impl Identifier {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}
