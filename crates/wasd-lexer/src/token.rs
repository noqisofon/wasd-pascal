//! トークン種別の定義。
//!
//! ISO 7185準拠のトークンに加え、UCSD拡張のトークン（`UNIT`/`INTERFACE`/
//! `IMPLEMENTATION`/`USES`/`OTHERWISE`予約語、16進数リテラル、コンパイラ
//! ディレクティブ）も**区別なく**通常のトークンとして認識する。dialectによる
//! 許可/拒否の判定は行わない（`wasd_ast::Dialect`のドキュメント参照）。

use wasd_ast::Span;

/// 字句解析で生成される1トークン。
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// トークンの種別。
///
/// # 設計判断: `CompilerDirective`に`Span`を持たせない
///
/// タスク仕様では `CompilerDirective { name: String, args: String, span: Span }`
/// という形が示されているが、`Token`は既に全種別共通で`span`フィールドを
/// 持っているため、`TokenKind`側にも`span`を重複して持たせると情報が
/// 二重管理になる。ここでは他のバリアントとの一貫性を優先し、
/// `Token::span`を使う設計とした（`TokenKind`自体はspanを持たない）。
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // --- 予約語（ISO 7185, 35語すべて） ---
    Program,
    Begin,
    End,
    Var,
    Const,
    Type,
    Procedure,
    Function,
    If,
    Then,
    Else,
    While,
    Do,
    Repeat,
    Until,
    For,
    To,
    DownTo,
    Case,
    Of,
    Record,
    Array,
    Set,
    File,
    Packed,
    Label,
    Goto,
    With,
    Nil,
    Not,
    And,
    Or,
    Div,
    Mod,
    In,

    // --- 予約語（UCSD拡張） ---
    Unit,
    Interface,
    Implementation,
    Uses,
    Otherwise,

    // --- 識別子・リテラル ---
    Identifier(String),
    IntegerLiteral(i64),
    RealLiteral(f64),
    /// 文字列リテラル。`''`によるシングルクォートのエスケープは解決済みの値を持つ。
    ///
    /// # 設計判断: 文字リテラルを別種別に分けない
    ///
    /// ISO 7185の文法上、`'x'`（1文字）と`'xyz'`（複数文字）はどちらも
    /// 同一の構文要素 `character-string` であり、字句レベルでは区別されない。
    /// 長さ1の文字列を`char`型として扱うかどうかは意味解析の仕事であるため、
    /// レキサでは`StringLiteral`ひとつに統一する。
    StringLiteral(String),

    // --- 記号 ---
    Assign,     // :=
    Le,         // <=
    Ge,         // >=
    Ne,         // <>
    DotDot,     // ..
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Eq,         // =
    Lt,         // <
    Gt,         // >
    LParen,     // (
    RParen,     // )
    LBracket,   // [
    RBracket,   // ]
    Dot,        // .
    Comma,      // ,
    Semicolon,  // ;
    Colon,      // :
    Caret,      // ^

    /// UCSD拡張: コンパイラディレクティブ `(*$I foo.pas*)` のようなコメント風構文。
    /// 通常コメントとは区別し、`name`（`$`直後の英数字の並び。例: `"I"`）と
    /// `args`（`name`より後、`*)`より前の残り。前後の空白は取り除く）を保持する。
    ///
    /// UNCONFIRMED: `name`と`args`の区切り方（空白必須か、`name`が複数文字の
    /// ディレクティブ名を許すかなど）は一次資料で未確認。ここでは`$`直後の
    /// 連続した英数字を`name`として貪欲に取り、以降の空白を読み飛ばした残りを
    /// `args`とする実装にしている。単一文字のディレクティブコード
    /// （`$I`, `$U`, `$S`など）を想定した設計であり、将来一次資料で
    /// 異なる文法が確認された場合は要修正。
    CompilerDirective { name: String, args: String },

    /// ソース終端を表す番兵トークン。`wasd-parser`が先読みしやすいように含める。
    Eof,
}

/// 予約語の照合を行う。
///
/// # 設計判断: 予約語判定はcase-insensitiveに行い、識別子の表記は保持する
///
/// Pascalは本来case-insensitiveな言語であり、`Begin`/`BEGIN`/`begin`は
/// すべて同じ予約語として扱われる。一方で識別子（変数名など）は
/// ユーザーが書いた表記をそのまま保持したい（診断メッセージやLSPの
/// ホバー表示で元の大文字小文字を尊重するため）。
///
/// そのため、字句解析時には「小文字化した文字列で予約語テーブルを引く」
/// が「トークンの`Identifier`に格納する文字列はソース上の元の表記のまま」
/// という方針を採る。予約語だった場合は元の表記を捨てて対応する
/// `TokenKind`バリアントに変換するため、大文字小文字混在の予約語は
/// 常に同一のトークン種別として比較可能になる。
pub(crate) fn lookup_keyword(ident: &str) -> Option<TokenKind> {
    use TokenKind::*;

    // ISO 7185予約語・UCSD拡張予約語はいずれもASCIIのみなので、
    // ASCII限定の小文字化で十分（Unicodeの大文字小文字変換は不要）。
    let lower = ident.to_ascii_lowercase();
    let kind = match lower.as_str() {
        "program" => Program,
        "begin" => Begin,
        "end" => End,
        "var" => Var,
        "const" => Const,
        "type" => Type,
        "procedure" => Procedure,
        "function" => Function,
        "if" => If,
        "then" => Then,
        "else" => Else,
        "while" => While,
        "do" => Do,
        "repeat" => Repeat,
        "until" => Until,
        "for" => For,
        "to" => To,
        "downto" => DownTo,
        "case" => Case,
        "of" => Of,
        "record" => Record,
        "array" => Array,
        "set" => Set,
        "file" => File,
        "packed" => Packed,
        "label" => Label,
        "goto" => Goto,
        "with" => With,
        "nil" => Nil,
        "not" => Not,
        "and" => And,
        "or" => Or,
        "div" => Div,
        "mod" => Mod,
        "in" => In,

        // UCSD拡張予約語
        "unit" => Unit,
        "interface" => Interface,
        "implementation" => Implementation,
        "uses" => Uses,
        "otherwise" => Otherwise,

        _ => return None,
    };
    Some(kind)
}
