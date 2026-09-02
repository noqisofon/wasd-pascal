//! `wasd_ast::Span`（バイトオフセット範囲）からLSPの`Position`/`Range`
//! （0始まりの行番号 + UTF-16コードユニット単位の列オフセット）への変換。
//!
//! LSPの仕様上、`Position.character`はデフォルトでUTF-16コードユニット単位
//! であり（`positionEncoding`をネゴシエートしていないため、このサーバーは
//! 常にこのデフォルトに従う）、Rustの`char`単位やバイト単位とは異なる。
//! 日本語コメント等マルチバイト文字（BMP内であれば1文字=1 UTF-16コード
//! ユニット）や、稀にBMP外の文字（サロゲートペア=2 UTF-16コードユニット）
//! を含むソースでも位置がずれないよう、変換はUTF-16単位で行毎に計算する。

use tower_lsp::lsp_types::{Position, Range};
use wasd_driver::Span;

/// `source`中のバイトオフセット`byte_offset`を、0始まりの行番号とその行内
/// UTF-16コードユニットオフセットへ変換する。
///
/// `byte_offset`が`source`の範囲外（末尾のEOFに割り当てられた`Span`など）
/// を指す場合は、`source`の末尾にクランプする。`byte_offset`が文字境界上に
/// ない場合は、その直前の文字境界にクランプする（レキサ・パーサーが生成する
/// `Span`は常に文字境界上にある想定だが、防御的に扱う）。
pub fn offset_to_position(source: &str, byte_offset: usize) -> Position {
    let mut offset = byte_offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }

    let mut line: u32 = 0;
    let mut line_start = 0usize;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            line_start = i + ch.len_utf8();
        }
    }

    let character = source[line_start..offset]
        .chars()
        .map(|ch| ch.len_utf16() as u32)
        .sum();

    Position::new(line, character)
}

/// [`Span`]をLSPの`Range`へ変換する。
pub fn span_to_range(source: &str, span: Span) -> Range {
    let start = offset_to_position(source, span.start as usize);
    let end = offset_to_position(source, span.end as usize);
    Range::new(start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_only_source_maps_byte_offsets_to_line_and_character() {
        let source = "PROGRAM Foo;\nBEGIN\nEND.\n";
        // "Foo" starts at byte 8, ends at byte 11.
        let pos = offset_to_position(source, 8);
        assert_eq!(pos, Position::new(0, 8));

        let pos_end = offset_to_position(source, 11);
        assert_eq!(pos_end, Position::new(0, 11));

        // "END" is on line 2 (0-based), starting at byte 19.
        let end_offset = source.find("END").unwrap();
        let pos = offset_to_position(source, end_offset);
        assert_eq!(pos, Position::new(2, 0));
    }

    #[test]
    fn multibyte_comment_does_not_shift_later_ascii_positions() {
        // "こんにちは" is 5 characters, each within the BMP (1 UTF-16 unit
        // each) but 3 bytes each in UTF-8, so byte and UTF-16 offsets
        // diverge sharply here.
        let source = "{ こんにちは }\nVAR x: INTEGER;\n";
        let x_byte_offset = source.find('x').unwrap();
        let pos = offset_to_position(source, x_byte_offset);
        // Line 1 (0-based), "VAR " is 4 UTF-16 units in.
        assert_eq!(pos, Position::new(1, 4));
    }

    #[test]
    fn astral_character_counts_as_two_utf16_code_units() {
        // U+1F600 (😀) lies outside the BMP: 4 bytes in UTF-8, but 2
        // UTF-16 code units (a surrogate pair).
        let source = "{ 😀 }\nx";
        let x_byte_offset = source.find('x').unwrap();
        let pos = offset_to_position(source, x_byte_offset);
        // "{ " (2) + the emoji (2 UTF-16 units) + " }" (2) = 6 units on line 0.
        assert_eq!(pos, Position::new(1, 0));

        let emoji_byte_offset = source.find('😀').unwrap();
        let pos = offset_to_position(source, emoji_byte_offset);
        assert_eq!(pos, Position::new(0, 2));
    }

    #[test]
    fn clamps_an_offset_past_the_end_of_source() {
        let source = "PROGRAM Foo;";
        let pos = offset_to_position(source, 1000);
        assert_eq!(pos, Position::new(0, source.chars().count() as u32));
    }

    #[test]
    fn span_to_range_converts_both_endpoints() {
        let source = "PROGRAM Foo;\nBEGIN\nEND.\n";
        let span = Span::new(8, 11);
        let range = span_to_range(source, span);
        assert_eq!(range.start, Position::new(0, 8));
        assert_eq!(range.end, Position::new(0, 11));
    }
}
