//! `wasdc`バイナリのエンドツーエンドテスト。
//!
//! `assert_cmd`で実際にバイナリを起動し、標準出力・終了コードを検証する。
//! リポジトリルートの`examples/`にあるサンプルファイルをそのまま入力として使う。

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join(name)
}

fn wasdc() -> Command {
    Command::cargo_bin("wasdc").expect("wasdc binary should build")
}

/// Step 15: `hello.pas`（`WriteLn('Hello, world!')`を含む）は、dialectを
/// 問わず意味解析上の警告もエラーも出さずに`check`が成功すること
/// （以前は多文字の文字列リテラルに対して`Severity::Warning`を出していた
/// が、その暫定処理は撤廃した。`crates/wasd-sema/src/typeck.rs`の
/// `infer_string_literal_type`ドキュメント参照）。
#[test]
fn check_succeeds_on_a_valid_program() {
    wasdc()
        .arg("check")
        .arg(example("hello.pas"))
        .assert()
        .success()
        .stdout(predicate::str::contains("OK: no errors found"))
        .stdout(predicate::str::contains("warning:").not());
}

#[test]
fn check_exits_nonzero_on_a_type_error() {
    wasdc()
        .arg("check")
        .arg(example("type_error.pas"))
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("error:"))
        .stdout(predicate::str::contains("type_error.pas"));
}

#[test]
fn check_reports_a_dialect_error_without_std_ucsd() {
    wasdc()
        .arg("check")
        .arg(example("ucsd_unit.pas"))
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("UCSD"));
}

#[test]
fn check_succeeds_on_ucsd_unit_with_std_ucsd() {
    wasdc()
        .arg("check")
        .arg(example("ucsd_unit.pas"))
        .arg("--std=ucsd")
        .assert()
        .success()
        .stdout(predicate::str::contains("OK: no errors found"));
}

#[test]
fn parse_emits_ast_when_requested() {
    wasdc()
        .arg("parse")
        .arg(example("procedures.pas"))
        .arg("--emit-ast")
        .assert()
        .success()
        .stdout(predicate::str::contains("Program {"))
        .stdout(predicate::str::contains("Factorial"));
}

#[test]
fn parse_without_emit_ast_only_prints_diagnostics() {
    wasdc()
        .arg("parse")
        .arg(example("procedures.pas"))
        .assert()
        .success()
        .stdout(predicate::str::contains("OK: no errors found"))
        .stdout(predicate::str::contains("Program {").not());
}

#[test]
fn compile_emits_pcode_for_a_minimal_scope_program() {
    wasdc()
        .arg("compile")
        .arg(example("pcode_minimal.pas"))
        .arg("--emit-pcode")
        .assert()
        .success()
        .stdout(predicate::str::contains("OK: no errors found"))
        .stdout(predicate::str::contains("STP"));
}

#[test]
fn compile_without_emit_pcode_only_prints_diagnostics() {
    wasdc()
        .arg("compile")
        .arg(example("pcode_minimal.pas"))
        .assert()
        .success()
        .stdout(predicate::str::contains("OK: no errors found"))
        .stdout(predicate::str::contains("STP").not());
}

#[test]
fn compile_emits_pcode_for_a_program_with_procedures_and_functions() {
    wasdc()
        .arg("compile")
        .arg(example("pcode_procedures.pas"))
        .arg("--emit-pcode")
        .assert()
        .success()
        .stdout(predicate::str::contains("OK: no errors found"))
        .stdout(predicate::str::contains("CPG"))
        .stdout(predicate::str::contains("RPU"));
}

/// Step 15: `hello.pas`は文字列リテラルを`WriteLn`へ直接渡すだけなので
/// もはや"out of scope"の例として使えない（`check_succeeds_on_a_valid_program`
/// 参照）。`pcode_unsupported.pas`（ポインタ型の`VAR`宣言。意味解析は通るが
/// `wasd-pcode`のスコープ外）に差し替える。Step 19で配列型、Step 20で
/// レコード型のグローバル`VAR`宣言がそれぞれサポート対象になったため、
/// `pcode_unsupported.pas`自体の中身もポインタ型に差し替えた（配列型・
/// レコード型はもはや"out of scope"の例として使えない）。
#[test]
fn compile_reports_out_of_scope_constructs_without_panicking() {
    wasdc()
        .arg("compile")
        .arg(example("pcode_unsupported.pas"))
        .arg("--emit-pcode")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("out of scope"))
        .stdout(predicate::str::contains("<no p-code:"));
}

#[test]
fn run_executes_a_minimal_scope_program_and_prints_globals() {
    wasdc()
        .arg("run")
        .arg(example("pcode_minimal.pas"))
        .assert()
        .success()
        .stdout(predicate::str::contains("OK: program ran to completion"))
        .stdout(predicate::str::contains("[0] = 55"));
}

#[test]
fn run_executes_procedures_and_functions_including_recursion_and_var_params() {
    // Factorial(5) = 120, then Increment(VAR result) makes it 121.
    wasdc()
        .arg("run")
        .arg(example("pcode_procedures.pas"))
        .assert()
        .success()
        .stdout(predicate::str::contains("OK: program ran to completion"))
        .stdout(predicate::str::contains("[0] = 121"));
}

/// Step 15のゴール: `wasdc run examples/hello.pas`が実際に
/// `Hello, world!`と出力すること。
#[test]
fn run_executes_hello_and_prints_hello_world() {
    wasdc()
        .arg("run")
        .arg(example("hello.pas"))
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello, world!\n42\n"))
        .stdout(predicate::str::contains("OK: program ran to completion"));
}

#[test]
fn run_executes_writeln_demo_and_prints_output() {
    wasdc()
        .arg("run")
        .arg(example("writeln_demo.pas"))
        .assert()
        .success()
        .stdout(predicate::str::contains("55\ntrue\n\n"))
        .stdout(predicate::str::contains("OK: program ran to completion"));
}

/// Step 15: `compile_reports_out_of_scope_constructs_without_panicking`と
/// 同じ理由で`hello.pas`から`pcode_unsupported.pas`（Step 20時点ではポインタ
/// 型の`VAR`宣言）に差し替える。
#[test]
fn run_reports_out_of_scope_constructs_without_panicking() {
    wasdc()
        .arg("run")
        .arg(example("pcode_unsupported.pas"))
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("out of scope"))
        .stdout(predicate::str::contains("<not run:"));
}

/// Step 16: `string_test.pas`（`STRING[80]`変数へ文字列リテラルを代入し
/// `WriteLn`する）は、`ucsd_unit.pas`と同様UCSD拡張構文を使うため
/// `--std=ucsd`が必要。
#[test]
fn check_succeeds_on_string_test_with_std_ucsd() {
    wasdc()
        .arg("check")
        .arg(example("string_test.pas"))
        .arg("--std=ucsd")
        .assert()
        .success()
        .stdout(predicate::str::contains("OK: no errors found"));
}

/// Step 16のゴール: `wasdc run --std=ucsd examples/string_test.pas`が実際に
/// `Hello, world!`と出力すること。
#[test]
fn run_executes_string_test_and_prints_hello_world() {
    wasdc()
        .arg("run")
        .arg(example("string_test.pas"))
        .arg("--std=ucsd")
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello, world!\n"))
        .stdout(predicate::str::contains("OK: program ran to completion"));
}

/// Step 17: 引数なし`PROCEDURE`呼び出しの最小スコープ動作確認。
/// `Greet;`という文で呼び出された`PROCEDURE Greet`が実際に
/// `WriteLn('Hello from a procedure!')`を実行し、制御が呼び出し元へ
/// 正しく戻って`WriteLn('Back in main.')`も実行されること。
#[test]
fn run_executes_argument_less_procedure_call() {
    wasdc()
        .arg("run")
        .arg(example("procedure_call_minimal.pas"))
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Hello from a procedure!\nBack in main.\n",
        ))
        .stdout(predicate::str::contains("OK: program ran to completion"));
}

/// Step 17: 同じ引数なし`PROCEDURE`を複数回呼び出しても、呼び出しごとに
/// MSCW/スタックが正しく積み直され、都度同じ本体が実行されること。
#[test]
fn run_executes_the_same_argument_less_procedure_repeatedly() {
    wasdc()
        .arg("run")
        .arg(example("procedure_call_repeated.pas"))
        .assert()
        .success()
        .stdout(predicate::str::contains("Hi!\nHi!\nHi!\n"))
        .stdout(predicate::str::contains("OK: program ran to completion"));
}

/// Step 18のゴール: `wasdc run --std=ucsd examples/func_test.pas`が実際に
/// `FUNCTION`の`INTEGER`値仮引数+戻り値（`Double(21) = 42`）と、
/// `PROCEDURE`の`STRING[n]`値仮引数（`PrintGreeting('Hello from a
/// parameter!')`）の両方を正しく実行すること。`STRING[n]`はUCSD拡張なので
/// `--std=ucsd`が必要（`string_test.pas`と同じ理由）。
#[test]
fn run_executes_function_and_string_parameter_sample() {
    wasdc()
        .arg("run")
        .arg(example("func_test.pas"))
        .arg("--std=ucsd")
        .assert()
        .success()
        .stdout(predicate::str::contains("42\nHello from a parameter!\n"))
        .stdout(predicate::str::contains("OK: program ran to completion"));
}

#[test]
fn missing_file_exits_with_io_error_code() {
    wasdc()
        .arg("check")
        .arg("examples/does_not_exist.pas")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("failed to read"));
}
