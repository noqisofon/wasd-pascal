//! テスト共通ヘルパー: Pascalソース文字列からp-codeを生成する。
//! `crates/wasd-pcode/tests/codegen.rs`と同様、レキサ→パーサー→p-code生成
//! のみを通す（意味解析エラーの検証はこのクレートの関心事ではない）。

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
