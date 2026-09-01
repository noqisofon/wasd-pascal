//! レキサ→パーサー→意味解析のパイプライン呼び出し。
//!
//! `wasdc`（CLI）とLSPサーバーの双方が、ここに集約された[`compile`]を
//! 呼び出すだけでソース文字列から診断とASTを得られるようにする。
//! p-code生成（`wasd-pcode`）はまだ無いため、このパイプラインは
//! 意味解析までで止まる。

use std::path::PathBuf;

use wasd_ast::{CompilationUnit, Diagnostic, Dialect, Program, Unit};
use wasd_lexer::Lexer;
use wasd_parser::Parser;
use wasd_sema::SemaContext;

/// [`compile`]の挙動を制御するオプション。
pub struct CompileOptions {
    /// 有効化するdialect。デフォルトはISO 7185準拠の標準Pascal。
    pub dialect: Dialect,
    /// 診断メッセージでのファイル名表示用。ソースが実際のファイルに
    /// 由来しない場合（LSPの未保存バッファ等）は`None`でよい。
    pub source_path: Option<PathBuf>,
}

impl CompileOptions {
    pub fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            source_path: None,
        }
    }

    pub fn with_source_path(mut self, path: PathBuf) -> Self {
        self.source_path = Some(path);
        self
    }
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self::new(Dialect::default())
    }
}

/// [`compile`]の結果。`program`と`unit`はどちらか一方のみ`Some`になる
/// （ソースが`PROGRAM`と`UNIT`のどちらであったかによる）。レキサ・パーサーが
/// 致命的に失敗しASTをまったく構築できなかった場合は両方とも`None`になる。
pub struct CompileResult {
    pub program: Option<Program>,
    pub unit: Option<Unit>,
    pub diagnostics: Vec<Diagnostic>,
}

/// ソース文字列を1本のコンパイル単位としてコンパイルする。
///
/// レキサ・パーサー・意味解析の各フェーズを順に呼び出し、フェーズを
/// 跨いで`Diagnostic`を蓄積する。各フェーズはエラーが出ても可能な限り
/// 処理を継続する設計（各クレートのドキュメント参照）だが、パーサーが
/// 完全に何もASTを構築できなかった場合（空入力など）は、意味解析を
/// スキップする。
///
/// 返される診断は、ソース上の出現位置（`Span::start`）の昇順にソートされる。
pub fn compile(source: &str, options: &CompileOptions) -> CompileResult {
    let mut diagnostics = Vec::new();

    let mut lexer = Lexer::new(source);
    let (tokens, lex_diags) = lexer.tokenize();
    diagnostics.extend(lex_diags);

    let mut parser = Parser::new(tokens);
    let (compilation_unit, parse_diags) = parser.parse_compilation_unit();
    diagnostics.extend(parse_diags);

    let mut program = None;
    let mut unit = None;

    if let Some(compilation_unit) = compilation_unit {
        let mut sema = SemaContext::new(options.dialect);
        match compilation_unit {
            CompilationUnit::Program(p) => {
                diagnostics.extend(sema.check_program(&p));
                program = Some(p);
            }
            CompilationUnit::Unit(u) => {
                diagnostics.extend(sema.check_unit(&u));
                unit = Some(u);
            }
        }
    }

    diagnostics.sort_by_key(|d| d.span.start);

    CompileResult {
        program,
        unit,
        diagnostics,
    }
}
