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

#[test]
fn check_succeeds_on_a_valid_program() {
    wasdc()
        .arg("check")
        .arg(example("hello.pas"))
        .assert()
        .success()
        .stdout(predicate::str::contains("warning:"));
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

#[test]
fn compile_reports_out_of_scope_constructs_without_panicking() {
    wasdc()
        .arg("compile")
        .arg(example("hello.pas"))
        .arg("--emit-pcode")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("out of scope"))
        .stdout(predicate::str::contains("<no p-code:"));
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
