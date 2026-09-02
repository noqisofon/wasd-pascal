//! テスト共通ヘルパー: Pascalソース文字列からp-codeを生成する。
//! `crates/wasd-pcode/tests/codegen.rs`と同様、レキサ→パーサー→p-code生成
//! のみを通す（意味解析エラーの検証はこのクレートの関心事ではない）。

use std::io::Write;
use std::sync::{Arc, Mutex};

use wasd_ast::Program;
use wasd_lexer::Lexer;
use wasd_parser::Parser;
use wasd_pcode::{CodeGenerator, PCodeModule};

#[allow(dead_code)]
pub fn parse_program(source: &str) -> Program {
    let mut lexer = Lexer::new(source);
    let (tokens, lex_diags) = lexer.tokenize();
    assert!(lex_diags.is_empty(), "lexer diagnostics: {lex_diags:?}");

    let mut parser = Parser::new(tokens);
    let (program, parse_diags) = parser.parse_program();
    assert!(
        parse_diags.is_empty(),
        "parser diagnostics: {parse_diags:?}"
    );
    program.expect("source should parse into a Program")
}

#[allow(dead_code)]
pub fn compile(source: &str) -> PCodeModule {
    let program = parse_program(source);
    CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed")
}

/// `PMachine::with_output`へ渡せる、キャプチャ可能な`Write`実装。
/// `Arc<Mutex<Vec<u8>>>`を介して`PMachine`実行後もバッファの内容を読める
/// （`PMachine`自体は出力先を所有してしまう`Box<dyn Write>`として受け取る
/// ため、`PMachine`の外からも中身を覗けるようにこの`Arc`越しの共有にして
/// ある。`WriteLn`の出力をテストで検証するために使う）。
#[derive(Clone, Default)]
#[allow(dead_code)]
pub struct CapturedOutput(Arc<Mutex<Vec<u8>>>);

#[allow(dead_code)]
impl CapturedOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// これまでに書き込まれた内容をUTF-8文字列として返す。
    pub fn as_string(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).expect("output should be valid UTF-8")
    }
}

impl Write for CapturedOutput {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
