//! p-code生成（[`crate::pcode::compile_to_pcode`]）に続けて、
//! `pmachine-core`で実際に実行するところまでを一気通貫で行う。
//!
//! `wasdc run <file>`（`wasd-cli`）から使われる。Step 14から`WriteLn`
//! （INTEGER/BOOLEAN・0/1引数のみ）、Step 15から文字列リテラルも
//! （`crates/wasd-pcode/src/codegen.rs`の`gen_writeln_call`のドキュメント
//! 参照）実際に標準出力へ書き込まれる
//! （`pmachine_core::PMachine::new`が標準出力へ直結する簡易実装のため）。
//! `WriteLn`以外の組み込み手続き（`Write`/`Read`/`ReadLn`/`New`/`Dispose`）は
//! 引き続きスコープ外。実行結果（`WriteLn`の出力とは別に）グローバル変数の
//! スナップショットも得られる（デバッグ目的。[`RunResult`]のドキュメント
//! 参照）。

use pmachine_core::{PMachine, RuntimeError};
use wasd_ast::Diagnostic;

use crate::compile::CompileOptions;
use crate::pcode::compile_to_pcode;

/// [`run`]の結果。
pub struct RunResult {
    /// レキサ・パーサー・意味解析・p-code生成を通じた診断の集合。
    pub diagnostics: Vec<Diagnostic>,
    /// p-code生成にすら失敗した場合は`None`（`diagnostics`にエラーが
    /// 含まれる）。
    pub executed: bool,
    /// 実行時エラー（0除算等）。生成に失敗した場合や、実行が正常に
    /// `STP`まで到達した場合は`None`。
    pub runtime_error: Option<RuntimeError>,
    /// 実行終了時点（正常終了・実行時エラーのいずれでも）のグローバル
    /// データ領域のスナップショット。p-code生成に失敗した場合は空。
    pub globals: Vec<i16>,
}

/// ソース文字列をコンパイルし、p-code生成に成功すれば
/// `pmachine-core::PMachine`で実行する。
pub fn run(source: &str, options: &CompileOptions) -> RunResult {
    let emit_result = compile_to_pcode(source, options);

    let Some(pcode) = emit_result.pcode else {
        return RunResult {
            diagnostics: emit_result.diagnostics,
            executed: false,
            runtime_error: None,
            globals: Vec::new(),
        };
    };

    let mut vm = PMachine::new(pcode);
    let runtime_error = vm.run().err();

    RunResult {
        diagnostics: emit_result.diagnostics,
        executed: true,
        runtime_error,
        globals: vm.globals().to_vec(),
    }
}
