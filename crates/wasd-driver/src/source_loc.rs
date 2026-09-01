//! `Span`（バイトオフセット範囲）をソース上の行・列・該当行テキストへ
//! 変換するヘルパー。
//!
//! CLIの診断表示（`  --> file.pas:12:5`のような形式）とLSPの
//! `Position`/`Range`変換の両方から使われる想定のため、`wasd-driver`に
//! 置く（表示形式そのものはCLI/LSPそれぞれの責務）。

use wasd_ast::Span;

/// 1始まりの行・列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: usize,
    pub column: usize,
}

/// [`Span`]をソース中の位置へ変換した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub start: LineCol,
    pub end: LineCol,
    /// `start.line`に対応するソース行のテキスト（改行文字を含まない）。
    pub line_text: String,
}

/// `source`中の`span`が指す位置を、行・列・該当行テキストへ変換する。
///
/// `span`が`source`の範囲外を指す場合（レキサ・パーサーが末尾のEOFに
/// `Span`を割り当てるケースなど）は、`source`の末尾にクランプする。
pub fn locate(source: &str, span: Span) -> SourceLocation {
    let len = source.len();
    let start = (span.start as usize).min(len);
    let end = (span.end as usize).min(len).max(start);

    let start_lc = offset_to_line_col(source, start);
    let end_lc = offset_to_line_col(source, end);
    let line_text = line_text_for(source, start_lc.line);

    SourceLocation {
        start: start_lc,
        end: end_lc,
        line_text,
    }
}

fn offset_to_line_col(source: &str, offset: usize) -> LineCol {
    let mut line = 1usize;
    let mut column = 1usize;
    for ch in source[..offset].chars() {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    LineCol { line, column }
}

fn line_text_for(source: &str, line_number: usize) -> String {
    source
        .lines()
        .nth(line_number - 1)
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_a_span_on_the_first_line() {
        let source = "PROGRAM Foo;\nBEGIN\nEND.\n";
        let span = Span::new(8, 11); // "Foo"
        let loc = locate(source, span);
        assert_eq!(loc.start, LineCol { line: 1, column: 9 });
        assert_eq!(
            loc.end,
            LineCol {
                line: 1,
                column: 12
            }
        );
        assert_eq!(loc.line_text, "PROGRAM Foo;");
    }

    #[test]
    fn locates_a_span_on_a_later_line() {
        let source = "PROGRAM Foo;\nVAR\n    x: INTEGER;\nBEGIN\nEND.\n";
        // "x" on line 3.
        let offset = source.find('x').unwrap() as u32;
        let span = Span::new(offset, offset + 1);
        let loc = locate(source, span);
        assert_eq!(loc.start.line, 3);
        assert_eq!(loc.line_text, "    x: INTEGER;");
    }

    #[test]
    fn clamps_a_span_past_the_end_of_source() {
        let source = "PROGRAM Foo;";
        let span = Span::new(100, 200);
        let loc = locate(source, span);
        assert_eq!(loc.start.line, 1);
        assert_eq!(loc.start.column, source.len() + 1);
    }
}
