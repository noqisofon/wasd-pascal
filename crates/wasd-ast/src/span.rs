//! ソースコード上の位置範囲。

/// ソースファイル中のバイトオフセット範囲 `[start, end)`。
///
/// 行・列への変換は表示側（CLI/LSP）の責務とし、`Span`自体はオフセットのみを持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}
