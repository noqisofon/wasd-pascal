//! 手書きの文字単位スキャナによるレキサ本体。
//!
//! エラー耐性を最優先する: 不正な文字や未終端の文字列・コメントに
//! 遭遇してもパニックせず、`Diagnostic`を蓄積しつつ可能な限り
//! トークン化を継続する（LSPでの利用を想定した設計）。

use wasd_ast::{Diagnostic, Severity, Span};

use crate::token::{lookup_keyword, Token, TokenKind};

pub struct Lexer<'src> {
    source: &'src str,
    /// 現在位置（バイトオフセット）。
    pos: usize,
    diagnostics: Vec<Diagnostic>,
}

impl<'src> Lexer<'src> {
    pub fn new(source: &'src str) -> Self {
        Self {
            source,
            pos: 0,
            diagnostics: Vec::new(),
        }
    }

    /// ソース全体をトークン化する。エラーがあってもパニックせず、
    /// `Diagnostic`を蓄積しつつ可能な限りトークン化を継続する。
    pub fn tokenize(&mut self) -> (Vec<Token>, Vec<Diagnostic>) {
        let mut tokens = Vec::new();

        loop {
            self.skip_whitespace_and_comments();

            let start = self.pos;
            let Some(c) = self.peek_char() else {
                tokens.push(Token::new(
                    TokenKind::Eof,
                    Span::new(start as u32, start as u32),
                ));
                break;
            };

            let kind = if is_ident_start(c) {
                self.scan_identifier_or_keyword()
            } else if c.is_ascii_digit() {
                self.scan_number()
            } else if c == '$' {
                self.scan_hex_literal()
            } else if c == '\'' {
                self.scan_string_literal()
            } else {
                self.scan_symbol()
            };

            if let Some(kind) = kind {
                let end = self.pos;
                tokens.push(Token::new(kind, Span::new(start as u32, end as u32)));
            }
            // `kind`が`None`の場合は不正文字などですでに`Diagnostic`を
            // 積んだ上でスキャン位置を進めているので、単に次のループへ進む。
        }

        (tokens, std::mem::take(&mut self.diagnostics))
    }

    // --- 低レベルな文字操作 ---

    fn rest(&self) -> &'src str {
        &self.source[self.pos..]
    }

    fn peek_char(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_char_at(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek_char()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn starts_with(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::new(span, Severity::Error, message));
    }

    // --- 空白・コメント ---

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek_char() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('{') => self.scan_brace_comment(),
                Some('(') if self.peek_char_at(1) == Some('*') => {
                    if self.peek_char_at(2) == Some('$') {
                        // `(*$...*)`はコンパイラディレクティブ。通常コメントとして
                        // 読み飛ばさず、ループを抜けてトークンスキャン経路
                        // （`scan_symbol` -> `scan_compiler_directive`）に委ねる。
                        break;
                    }
                    self.scan_paren_comment();
                }
                _ => break,
            }
        }
    }

    /// `{ ... }`形式のコメントを読み飛ばす。ネストはISO 7185では認められて
    /// いないため、最初に現れた`}`で終了とする。
    fn scan_brace_comment(&mut self) {
        let start = self.pos;
        self.bump(); // '{'
        loop {
            match self.peek_char() {
                Some('}') => {
                    self.bump();
                    return;
                }
                Some(_) => {
                    self.bump();
                }
                None => {
                    self.error(
                        Span::new(start as u32, self.pos as u32),
                        "unterminated comment (missing closing '}')",
                    );
                    return;
                }
            }
        }
    }

    /// `(* ... *)`形式のコメント。呼び出し元(`skip_whitespace_and_comments`)が
    /// 既に`(*$`（コンパイラディレクティブ）ではないことを確認済みの前提。
    fn scan_paren_comment(&mut self) {
        debug_assert!(self.starts_with("(*"));
        debug_assert_ne!(self.peek_char_at(2), Some('$'));

        let start = self.pos;
        self.bump(); // '('
        self.bump(); // '*'
        loop {
            if self.starts_with("*)") {
                self.bump();
                self.bump();
                return;
            }
            if self.bump().is_none() {
                self.error(
                    Span::new(start as u32, self.pos as u32),
                    "unterminated comment (missing closing '*)')",
                );
                return;
            }
        }
    }

    // --- 記号 ---

    fn scan_symbol(&mut self) -> Option<TokenKind> {
        let start = self.pos;
        let c = self.bump().expect("caller already peeked a char");

        let kind = match c {
            ':' => {
                if self.peek_char() == Some('=') {
                    self.bump();
                    TokenKind::Assign
                } else {
                    TokenKind::Colon
                }
            }
            '<' => match self.peek_char() {
                Some('=') => {
                    self.bump();
                    TokenKind::Le
                }
                Some('>') => {
                    self.bump();
                    TokenKind::Ne
                }
                _ => TokenKind::Lt,
            },
            '>' => {
                if self.peek_char() == Some('=') {
                    self.bump();
                    TokenKind::Ge
                } else {
                    TokenKind::Gt
                }
            }
            '.' => {
                if self.peek_char() == Some('.') {
                    self.bump();
                    TokenKind::DotDot
                } else {
                    TokenKind::Dot
                }
            }
            '(' => {
                // `(*`は`skip_whitespace_and_comments`で処理済みのはずだが、
                // `(*$`（コンパイラディレクティブ）だけはコメントとして
                // 読み飛ばされずここまで到達する。
                if self.peek_char() == Some('*') {
                    return self.scan_compiler_directive(start);
                }
                TokenKind::LParen
            }
            ')' => TokenKind::RParen,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ',' => TokenKind::Comma,
            ';' => TokenKind::Semicolon,
            '^' => TokenKind::Caret,
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '=' => TokenKind::Eq,
            other => {
                self.error(
                    Span::new(start as u32, self.pos as u32),
                    format!("unexpected character '{other}'"),
                );
                return None;
            }
        };
        Some(kind)
    }

    /// `(*$name args*)`形式のコンパイラディレクティブをスキャンする。
    /// 呼び出し元(`scan_symbol`)が既に`(`を消費済みで、現在位置は`*`を
    /// 指している前提（`(*$`の`(`のみ消費済み、`*$`は未消費）。
    fn scan_compiler_directive(&mut self, start: usize) -> Option<TokenKind> {
        self.bump(); // '*'
        self.bump(); // '$'

        let name_start = self.pos;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_alphanumeric() {
                self.bump();
            } else {
                break;
            }
        }
        let name = self.source[name_start..self.pos].to_string();

        // name直後の空白は区切りとして読み飛ばす。
        while matches!(self.peek_char(), Some(c) if c.is_whitespace() && c != '\n' && c != '\r') {
            self.bump();
        }

        let args_start = self.pos;
        loop {
            if self.starts_with("*)") {
                let args = self.source[args_start..self.pos].trim_end().to_string();
                self.bump();
                self.bump();
                return Some(TokenKind::CompilerDirective { name, args });
            }
            if self.bump().is_none() {
                self.error(
                    Span::new(start as u32, self.pos as u32),
                    "unterminated compiler directive (missing closing '*)')",
                );
                let args = self.source[args_start..self.pos].trim_end().to_string();
                return Some(TokenKind::CompilerDirective { name, args });
            }
        }
    }

    // --- 識別子・予約語 ---

    fn scan_identifier_or_keyword(&mut self) -> Option<TokenKind> {
        let start = self.pos;
        self.bump(); // 先頭の1文字（is_ident_startで確認済み）
        while matches!(self.peek_char(), Some(c) if is_ident_continue(c)) {
            self.bump();
        }
        let text = &self.source[start..self.pos];

        // UNCONFIRMED: 識別子の最大有効長（UCSD Pascalでは歴史的に
        // 8文字までが意味を持ち、それ以降は無視される実装が知られているが、
        // 一次資料（SofTech Internal Architecture Reference Manual等）で
        // 未確認のため、ここでは長さ制限を一切課さない。
        if let Some(kw) = lookup_keyword(text) {
            Some(kw)
        } else {
            Some(TokenKind::Identifier(text.to_string()))
        }
    }

    // --- 数値リテラル ---

    fn scan_number(&mut self) -> Option<TokenKind> {
        let start = self.pos;
        self.consume_digits();

        let mut is_real = false;

        // 小数部: '.'の直後が数字の場合のみ小数部として消費する。
        // `1..10`のような範囲演算子`..`と衝突しないための判定。
        if self.peek_char() == Some('.')
            && matches!(self.peek_char_at(1), Some(d) if d.is_ascii_digit())
        {
            is_real = true;
            self.bump(); // '.'
            self.consume_digits();
        }

        // 指数部: 'e'/'E' [ '+' | '-' ] digit-sequence
        // ISO 7185の unsigned-real 文法に従う。
        if matches!(self.peek_char(), Some('e') | Some('E')) {
            let mut lookahead = 1;
            if matches!(self.peek_char_at(lookahead), Some('+') | Some('-')) {
                lookahead += 1;
            }
            if matches!(self.peek_char_at(lookahead), Some(d) if d.is_ascii_digit()) {
                is_real = true;
                self.bump(); // 'e' / 'E'
                if matches!(self.peek_char(), Some('+') | Some('-')) {
                    self.bump();
                }
                self.consume_digits();
            }
            // 数字が続かない場合は指数部ではないので、'e'は消費せず
            // 数値リテラルはここで終わる（'e'は後続の識別子等として扱われる）。
        }

        let text = &self.source[start..self.pos];
        if is_real {
            match text.parse::<f64>() {
                Ok(value) => Some(TokenKind::RealLiteral(value)),
                Err(_) => {
                    self.error(
                        Span::new(start as u32, self.pos as u32),
                        format!("invalid real literal '{text}'"),
                    );
                    Some(TokenKind::RealLiteral(0.0))
                }
            }
        } else {
            match text.parse::<i64>() {
                Ok(value) => Some(TokenKind::IntegerLiteral(value)),
                Err(_) => {
                    self.error(
                        Span::new(start as u32, self.pos as u32),
                        format!("integer literal '{text}' out of range"),
                    );
                    Some(TokenKind::IntegerLiteral(0))
                }
            }
        }
    }

    fn consume_digits(&mut self) {
        while matches!(self.peek_char(), Some(c) if c.is_ascii_digit()) {
            self.bump();
        }
    }

    /// UCSD拡張: `$FF`形式の16進数リテラル。
    ///
    /// `IntegerLiteral`ではなく`HexIntegerLiteral`を返す（`TokenKind`の
    /// ドキュメント参照）。dialectチェックを`wasd-sema`で行うためには、
    /// 「10進数の`255`」と「16進数の`$FF`」を区別できる必要があるため。
    fn scan_hex_literal(&mut self) -> Option<TokenKind> {
        let start = self.pos;
        self.bump(); // '$'

        let digits_start = self.pos;
        while matches!(self.peek_char(), Some(c) if c.is_ascii_hexdigit()) {
            self.bump();
        }
        let digits = &self.source[digits_start..self.pos];

        if digits.is_empty() {
            self.error(
                Span::new(start as u32, self.pos as u32),
                "empty hexadecimal literal after '$'",
            );
            return Some(TokenKind::HexIntegerLiteral(0));
        }

        match i64::from_str_radix(digits, 16) {
            Ok(value) => Some(TokenKind::HexIntegerLiteral(value)),
            Err(_) => {
                self.error(
                    Span::new(start as u32, self.pos as u32),
                    format!("hexadecimal literal '${digits}' out of range"),
                );
                Some(TokenKind::HexIntegerLiteral(0))
            }
        }
    }

    // --- 文字列リテラル ---

    /// `'...'`形式の文字列リテラル。`''`は1つのシングルクォート文字に
    /// エスケープされる（ISO 7185の`character-string`の仕様どおり）。
    fn scan_string_literal(&mut self) -> Option<TokenKind> {
        let start = self.pos;
        self.bump(); // 開始の '\''

        let mut value = String::new();
        loop {
            match self.peek_char() {
                Some('\'') => {
                    self.bump();
                    if self.peek_char() == Some('\'') {
                        // '' -> 単一の ' としてエスケープ
                        self.bump();
                        value.push('\'');
                    } else {
                        return Some(TokenKind::StringLiteral(value));
                    }
                }
                Some(c) => {
                    // UNCONFIRMED: 文字列リテラル中の改行の扱い（許容するか、
                    // エラーとすべきか)は一次資料で未確認。ここではエラー耐性を
                    // 優先し、改行を含め任意の文字をそのまま許容する。
                    self.bump();
                    value.push(c);
                }
                None => {
                    self.error(
                        Span::new(start as u32, self.pos as u32),
                        "unterminated string literal (missing closing '\\'')",
                    );
                    return Some(TokenKind::StringLiteral(value));
                }
            }
        }
    }
}

/// 識別子の先頭に使える文字か。
///
/// UNCONFIRMED: アンダースコア(`_`)を識別子の一部として許容するかは
/// UCSD Pascalの一次資料で未確認。ISO 7185は識別子を
/// `letter { letter | digit }` とのみ定義しておりアンダースコアを含まない
/// ため、ここでは含めない（後発のPascal方言での慣習を安易に持ち込まない）。
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        let (tokens, diagnostics) = Lexer::new(source).tokenize();
        assert!(
            diagnostics.is_empty(),
            "expected no diagnostics for {source:?}, got {diagnostics:?}"
        );
        tokens.into_iter().map(|t| t.kind).collect()
    }

    /// テスト対象(1): 基本的なISO 7185プログラムのトークン化。
    #[test]
    fn tokenizes_minimal_iso7185_program() {
        let source = "program Hello;\nbegin\n  writeln('hi')\nend.\n";
        let kinds = kinds(source);
        assert_eq!(
            kinds,
            vec![
                TokenKind::Program,
                TokenKind::Identifier("Hello".to_string()),
                TokenKind::Semicolon,
                TokenKind::Begin,
                TokenKind::Identifier("writeln".to_string()),
                TokenKind::LParen,
                TokenKind::StringLiteral("hi".to_string()),
                TokenKind::RParen,
                TokenKind::End,
                TokenKind::Dot,
                TokenKind::Eof,
            ]
        );
    }

    /// テスト対象(2): UCSD拡張トークン（UNIT/INTERFACEと16進数リテラル）が
    /// dialectエラーなしに通常のトークンとして認識されること。
    ///
    /// # Step 7での変更: `IntegerLiteral`ではなく`HexIntegerLiteral`を期待する
    ///
    /// 以前は16進数リテラルも通常の`IntegerLiteral`としてデコードするだけ
    /// だったが、`wasd-sema`でのdialectチェック導入にあたり「10進数の`26`」と
    /// 「16進数の`$1A`」を区別する必要が生じたため、専用の`HexIntegerLiteral`
    /// を返すようにした（`TokenKind::HexIntegerLiteral`のドキュメント参照）。
    #[test]
    fn recognizes_ucsd_extension_tokens_without_dialect_error() {
        let kinds = kinds("unit Foo;\ninterface\nconst x = $1A;\n");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Unit,
                TokenKind::Identifier("Foo".to_string()),
                TokenKind::Semicolon,
                TokenKind::Interface,
                TokenKind::Const,
                TokenKind::Identifier("x".to_string()),
                TokenKind::Eq,
                TokenKind::HexIntegerLiteral(0x1A),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    /// テスト対象(3): 両形式のコメント（`{ }` と `(* *)`）が正しく無視されること。
    #[test]
    fn ignores_both_comment_styles() {
        let kinds = kinds("var { this is a comment } x (* another comment *) : integer;");
        assert_eq!(
            kinds,
            vec![
                TokenKind::Var,
                TokenKind::Identifier("x".to_string()),
                TokenKind::Colon,
                TokenKind::Identifier("integer".to_string()),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    /// テスト対象(4): コンパイラディレクティブが通常コメントと区別されること。
    #[test]
    fn recognizes_compiler_directive_distinct_from_comment() {
        let kinds = kinds("(*$I foo.pas*)");
        assert_eq!(
            kinds,
            vec![
                TokenKind::CompilerDirective {
                    name: "I".to_string(),
                    args: "foo.pas".to_string(),
                },
                TokenKind::Eof,
            ]
        );
    }

    /// テスト対象(5): 文字列リテラル内の`''`エスケープ。
    #[test]
    fn handles_quote_escaping_in_string_literal() {
        let kinds = kinds("'it''s'");
        assert_eq!(
            kinds,
            vec![TokenKind::StringLiteral("it's".to_string()), TokenKind::Eof,]
        );
    }

    /// テスト対象(6): 不正な文字に対してパニックせず`Diagnostic`を返し、
    /// トークン化が継続すること。
    #[test]
    fn recovers_from_illegal_character_without_panicking() {
        let (tokens, diagnostics) = Lexer::new("x := 1 @ 2;").tokenize();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);

        let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Identifier("x".to_string()),
                TokenKind::Assign,
                TokenKind::IntegerLiteral(1),
                // '@' はDiagnosticとして記録され、トークン列には含まれない。
                TokenKind::IntegerLiteral(2),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    /// テスト対象(7): 大文字小文字混在の予約語が同一トークンとして扱われること。
    #[test]
    fn keywords_are_case_insensitive() {
        assert_eq!(kinds("Begin")[0], TokenKind::Begin);
        assert_eq!(kinds("BEGIN")[0], TokenKind::Begin);
        assert_eq!(kinds("begin")[0], TokenKind::Begin);
        assert_eq!(kinds("BeGiN")[0], TokenKind::Begin);
    }

    #[test]
    fn tokenizes_real_literals_with_exponent() {
        let kinds = kinds("3.25 2.5e10 1e-3");
        assert_eq!(
            kinds,
            vec![
                TokenKind::RealLiteral(3.25),
                TokenKind::RealLiteral(2.5e10),
                TokenKind::RealLiteral(1e-3),
                TokenKind::Eof,
            ]
        );
    }

    /// `1..10`のような範囲式で、`.`が小数点と誤認されないこと。
    #[test]
    fn does_not_confuse_range_dotdot_with_decimal_point() {
        let kinds = kinds("1..10");
        assert_eq!(
            kinds,
            vec![
                TokenKind::IntegerLiteral(1),
                TokenKind::DotDot,
                TokenKind::IntegerLiteral(10),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn unterminated_string_literal_produces_diagnostic_and_continues() {
        let (tokens, diagnostics) = Lexer::new("x := 'abc").tokenize();
        assert_eq!(diagnostics.len(), 1);
        let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Identifier("x".to_string()),
                TokenKind::Assign,
                TokenKind::StringLiteral("abc".to_string()),
                TokenKind::Eof,
            ]
        );
    }
}
