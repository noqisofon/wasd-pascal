//! p-code生成（`wasd-pcode`）の呼び出し。
//!
//! [`crate::compile::compile`]はレキサ→パーサー→意味解析までで止まる
//! （そのモジュールのドキュメント参照）。本モジュールはその結果に
//! p-code生成を継ぎ足す。`wasd-pcode::CodeGenerator`が対応する
//! `PROGRAM`のみを対象とし、`UNIT`や、意味解析エラーが既にある
//! ソースに対してはp-code生成そのものを行わない（壊れた入力に対して
//! コード生成器を走らせても無意味な診断が増えるだけのため）。

use wasd_ast::{Diagnostic, Program, Severity, Unit};
use wasd_pcode::{CodeGenerator, PCodeModule};

use crate::compile::{compile, CompileOptions};

/// [`compile_to_pcode`]の結果。
pub struct PCodeEmitResult {
    pub program: Option<Program>,
    pub unit: Option<Unit>,
    /// レキサ・パーサー・意味解析・p-code生成を通じた診断の集合
    /// （ソース上の出現位置の昇順にソート済み）。
    pub diagnostics: Vec<Diagnostic>,
    /// p-code生成に成功した場合のみ`Some`。既に診断（エラー）がある場合や
    /// `UNIT`コンパイル単位の場合は`None`。
    pub pcode: Option<PCodeModule>,
}

/// ソース文字列をコンパイルし、意味解析にエラーが無ければ続けてp-code
/// 生成を行う。
pub fn compile_to_pcode(source: &str, options: &CompileOptions) -> PCodeEmitResult {
    let mut result = compile(source, options);
    let mut pcode = None;

    let has_errors = result
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error);

    if !has_errors {
        if let Some(program) = &result.program {
            match CodeGenerator::new().generate(program) {
                Ok(module) => pcode = Some(module),
                Err(gen_diags) => result.diagnostics.extend(gen_diags),
            }
        } else if let Some(unit) = &result.unit {
            result.diagnostics.push(Diagnostic::new(
                unit.span,
                Severity::Error,
                "p-code generation for UNIT compilation units is out of scope for this step's \
                 minimal codegen (only PROGRAM is supported)",
            ));
        }
    }

    result.diagnostics.sort_by_key(|d| d.span.start);

    PCodeEmitResult {
        program: result.program,
        unit: result.unit,
        diagnostics: result.diagnostics,
        pcode,
    }
}
