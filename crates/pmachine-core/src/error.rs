//! p-machine実行時エラー。
//!
//! lexer/parser/semaは「エラーがあっても可能な限り処理を継続する」方針
//! だったが（各クレートのドキュメント参照）、p-machineの実行はそうでは
//! ない。0除算・スタックオーバーフロー等の実行時エラーに遭遇したら
//! 即座に実行を停止し、[`RuntimeError`]を返す（実プログラムの実行系として
//! 当然の挙動。`crates/pmachine-core`実装指示のタスク2参照）。

use std::fmt;

/// [`crate::PMachine::step`]/[`crate::PMachine::run`]が返しうる実行時エラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// スタック（演算スタック・活性化レコード領域を含む、本クレートが
    /// 単一の`Vec<i16>`で表現する領域全体）がこれ以上伸長できない。
    /// 本クレートでは実際には容量上限を設けていないため、主に
    /// [`RuntimeError::AddressOutOfRange`]と併せて「アドレス値が`i16`に
    /// 収まらないほど深い呼び出し・大きいプログラム」を報告するために使う。
    StackOverflow,
    /// スタックから値をpopしようとしたが、popできる値がない
    /// （命令列が期待する数だけスタックに値が積まれていない場合。
    /// 本来はコード生成器のバグを示すはずだが、念のため実行系としても
    /// 検出しエラーにする）。
    StackUnderflow,
    /// `DVI`/`MOD`でTOS（除数）が0だった。一次資料に「TOSが0なら実行時
    /// エラー」と明記されている（`crates/pmachine-core`実装指示のタスク3
    /// 参照）。
    DivisionByZero,
    /// 未実装の命令・機能に遭遇した（今回のスコープ外の命令等）。本クレートは
    /// [`wasd_pcode::Opcode`]の全バリアントを網羅的にmatchするため、
    /// オペコードのバリアント自体が原因でこの変種が返ることはないが、
    /// `CXG`がKERNEL以外のセグメントを呼び出そうとした場合や、KERNEL
    /// エミュレーションが未知のprocedure番号を渡された場合（`crate::machine`の
    /// `call_external`/`call_builtin_kernel`参照）にも同じ変種を使う。
    UnimplementedOpcode(String),
    /// `IPC`が命令列の範囲外を指した（`STP`に到達せず命令列の終端を
    /// 越えた場合。コード生成器が必ず末尾に`STP`を発行する前提が崩れて
    /// いることを示す）。
    IpcOutOfBounds,
    /// `LOD`/`STR`/`LDA`のレベル差が現在の実行コンテキストで解決できない
    /// （活性化レコードが1つも無いのにレベル差>0を要求された等）。
    NoActiveFrame,
    /// アドレス値（活性化レコード内オフセット、またはスタック中の絶対
    /// 位置）が`i16`の表現範囲（-32768..=32767）に収まらない。本クレートは
    /// プロジェクト方針の16ビットワード幅（`crates/pmachine-core`実装指示の
    /// タスク1参照）に従い、`LDA`が積むアドレス自体も1ワード（`i16`）で
    /// 表現するため、これを超える巨大なプログラムは実行できない。
    AddressOutOfRange,
    /// `CPL`/`CPG`/`CPI`/`SCPI1`/`SCPI2`の呼び出し先アドレスに対応する
    /// [`wasd_pcode::RoutineMeta`]が[`wasd_pcode::PCodeModule::routines`]に
    /// 見つからなかった。コード生成器が生成した命令列と、それに付随する
    /// ルーチン表が矛盾していることを示す（通常は起こらないはずの内部
    /// エラー）。
    UnknownRoutine,
    /// `CXG`が呼び出したKERNELの組み込みエミュレーション（`WriteLn`）が、
    /// 出力先（[`crate::PMachine`]に注入された`Box<dyn Write>`）への書き込みに
    /// 失敗した。ホストの標準出力が閉じられている等、通常は起こらないはずの
    /// I/Oエラーを報告するために用意してある。
    Io(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::StackOverflow => write!(f, "stack overflow"),
            RuntimeError::StackUnderflow => write!(f, "stack underflow"),
            RuntimeError::DivisionByZero => write!(f, "division by zero"),
            RuntimeError::UnimplementedOpcode(op) => write!(f, "unimplemented opcode: {op}"),
            RuntimeError::IpcOutOfBounds => write!(f, "IPC ran past the end of the code"),
            RuntimeError::NoActiveFrame => {
                write!(f, "no active activation record for this level reference")
            }
            RuntimeError::AddressOutOfRange => {
                write!(f, "address does not fit in a 16-bit p-machine word")
            }
            RuntimeError::UnknownRoutine => {
                write!(f, "call target has no matching routine metadata")
            }
            RuntimeError::Io(message) => write!(f, "I/O error: {message}"),
        }
    }
}

impl std::error::Error for RuntimeError {}
