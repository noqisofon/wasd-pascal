//! p-code命令のIR表現。
//!
//! オペコード自体の定義（`Confirmed`/`Unconfirmed`の分離）は
//! [`crate::opcode`]を参照。このモジュールは命令列（[`PCodeModule`]）と
//! 個々の命令（[`Instruction`]）を扱う。

use wasd_ast::Span;

use crate::opcode::{CodeAddress, Opcode};

/// p-code命令列中の1命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub opcode: Opcode,
    /// この命令の生成元となったASTノードのソース位置（診断・デバッグ用）。
    pub span: Span,
}

/// `PROCEDURE`/`FUNCTION`1件分の、呼び出し規約に必要なメタデータ。
///
/// # 追加の経緯（pmachine-core向け）
///
/// 実機のp-machineは、`CPL`/`CPG`/`CPI`が呼び出し先の活性化レコードを
/// 組み立てる際に必要な情報（パラメータ語数・ローカル変数領域の語数等）を、
/// セグメント辞書中の「プロシージャ辞書」から`CURPROC`経由で間接的に
/// 引く（[`crate::opcode::ConfirmedOp::Cpl`]のドキュメントの「プロシージャ
/// 番号(UB)→プロシージャ辞書経由の間接参照」を参照。本IRは呼び出し先を
/// 直接[`CodeAddress`]として保持する簡略化を採用しているため、この間接
/// 参照の仕組み自体は再現していない）。
///
/// しかし、`pmachine-core`が実際に`CPL`/`CPG`/`CPI`を実行するには、
/// 呼び出し先の活性化レコードを組み立てるための最小限の情報
/// （パラメータの語数、ローカル変数・一時変数領域の語数、`FUNCTION`かどうか）
/// がどうしても必要になる。この情報はp-code命令列そのものには
/// （実機同様）含まれていないため、[`PCodeModule::routines`]という
/// 別テーブルとして持たせることにした。これは実機のプロシージャ辞書に
/// 相当する、本IR向けの簡略化された表現である。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutineMeta {
    /// 本体の先頭命令のアドレス（呼び出し命令の呼び出し先と一致する）。
    pub entry: CodeAddress,
    /// 仮引数のワード数（`VAR`仮引数もそれ以外も1ワードとして数える。
    /// [`crate::codegen::CodeGenerator`]の活性化レコードのレイアウトの
    /// ドキュメント参照）。
    pub param_count: u16,
    /// ローカル変数・一時変数領域のワード数（`DATASIZE`。`RPU`の`b`
    /// パラメータの計算に使われるのと同じ値）。
    pub data_size: u16,
    /// `FUNCTION`かどうか（真の場合、活性化レコード末尾に戻り値用の
    /// 1ワードが追加される）。
    pub is_func: bool,
}

/// 1つの`PROGRAM`から生成されたp-code命令列。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PCodeModule {
    pub instructions: Vec<Instruction>,
    /// グローバルデータ領域が必要とするワード数（ユーザー宣言の`VAR`に
    /// 加え、コード生成器が導入した一時変数を含む）。
    pub global_data_words: u16,
    /// `PROGRAM`直下に宣言された`PROCEDURE`/`FUNCTION`ごとのメタデータ
    /// （実機のプロシージャ辞書に相当。[`RoutineMeta`]のドキュメント参照）。
    /// エントリアドレス（[`RoutineMeta::entry`]）の昇順にソート済み。
    pub routines: Vec<RoutineMeta>,
    /// `PROGRAM ... BEGIN ... END.`本体（`PROCEDURE`/`FUNCTION`宣言では
    /// ない、プログラム自身の実行文）の先頭命令のアドレス。
    ///
    /// # pmachine-core向けに追加した経緯
    ///
    /// [`crate::codegen::CodeGenerator::generate`]は、`PROGRAM`直下に
    /// 宣言された`PROCEDURE`/`FUNCTION`の本体を**先に**、プログラム本体を
    /// **後で**生成する（相互再帰に対応するため。
    /// `crates/wasd-pcode/src/codegen.rs`のモジュールドキュメント参照）。
    /// そのため`instructions`のインデックス`0`は、`PROCEDURE`/`FUNCTION`が
    /// 1つでも宣言されていれば、プログラム本体ではなく最初のルーチンの
    /// 本体を指す。`pmachine-core`が実行を開始すべき位置（実機で言う
    /// プログラムの初期`IPC`）はここでしか判別できないため、明示的な
    /// フィールドとして持たせる。
    pub entry: CodeAddress,
}
