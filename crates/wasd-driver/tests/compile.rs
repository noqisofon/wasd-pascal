//! `wasd-driver`の`compile`関数に対する統合テスト。
//!
//! `wasd-driver`の公開APIのみを通して、レキサ→パーサー→意味解析の
//! パイプライン全体（正常系・型エラー・dialectエラー）を検証する。

use wasd_driver::{compile, compile_to_pcode, CompileOptions, Dialect, Severity};

#[test]
fn compiles_a_valid_program_end_to_end() {
    let source = r#"
        PROGRAM Hello;
        VAR
            answer: INTEGER;
        BEGIN
            answer := 42;
            WriteLn(answer)
        END.
    "#;

    let result = compile(source, &CompileOptions::default());

    assert!(result.program.is_some());
    assert!(result.unit.is_none());
    assert!(
        result.diagnostics.is_empty(),
        "expected no diagnostics, got {:?}",
        result.diagnostics
    );
}

#[test]
fn compile_to_pcode_generates_a_module_for_a_minimal_scope_program() {
    let source = r#"
        PROGRAM Sum;
        VAR
            total: INTEGER;
        BEGIN
            total := 0;
            IF total = 0 THEN
                total := 1
        END.
    "#;

    let result = compile_to_pcode(source, &CompileOptions::default());

    assert!(
        result.diagnostics.is_empty(),
        "expected no diagnostics, got {:?}",
        result.diagnostics
    );
    let pcode = result.pcode.expect("pcode should have been generated");
    assert!(!pcode.instructions.is_empty());
}

/// Step 12: `PROCEDURE`/`FUNCTION`呼び出しを含むプログラムでも
/// `compile_to_pcode`がp-codeを生成できること（`WriteLn`のような組み込み
/// 手続きを使わない限り）。
#[test]
fn compile_to_pcode_generates_a_module_for_a_program_with_procedures_and_functions() {
    let source = r#"
        PROGRAM Procedures;
        VAR
            result: INTEGER;

        FUNCTION Factorial(n: INTEGER): INTEGER;
        BEGIN
            IF n <= 1 THEN
                Factorial := 1
            ELSE
                Factorial := n * Factorial(n - 1)
        END;

        PROCEDURE Increment(VAR value: INTEGER);
        BEGIN
            value := value + 1
        END;

        BEGIN
            result := Factorial(5);
            Increment(result)
        END.
    "#;

    let result = compile_to_pcode(source, &CompileOptions::default());

    assert!(
        result.diagnostics.is_empty(),
        "expected no diagnostics, got {:?}",
        result.diagnostics
    );
    let pcode = result.pcode.expect("pcode should have been generated");
    let text = pcode.to_string();
    assert!(text.contains("CPG"), "expected a CPG call, got:\n{text}");
    assert!(text.contains("RPU"), "expected an RPU return, got:\n{text}");
    assert!(
        text.contains("LDA"),
        "expected an LDA (VAR arg address), got:\n{text}"
    );
}

/// `WriteLn`はStep 14からINTEGER/BOOLEAN、Step 15から文字列リテラルの
/// 0/1引数のみ実際に動作する（`crates/wasd-pcode/src/codegen.rs`の
/// `gen_writeln_call`のドキュメント参照）。それ以外（`CASE`文等）は
/// 引き続きこのクレートのスコープ外。
#[test]
fn compile_to_pcode_reports_out_of_scope_constructs_without_generating_pcode() {
    let source = r#"
        PROGRAM UsesCase;
        VAR
            answer: INTEGER;
        BEGIN
            answer := 42;
            CASE answer OF
                42: answer := 1
            END
        END.
    "#;

    let result = compile_to_pcode(source, &CompileOptions::default());

    assert!(result.pcode.is_none());
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error));
}

#[test]
fn reports_a_type_mismatch_between_boolean_and_integer() {
    let source = r#"
        PROGRAM TypeError;
        VAR
            flag: BOOLEAN;
            count: INTEGER;
        BEGIN
            flag := TRUE;
            count := flag + 1
        END.
    "#;

    let result = compile(source, &CompileOptions::default());

    assert!(result.program.is_some());
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert_eq!(errors.len(), 1, "diagnostics: {:?}", result.diagnostics);
    assert!(errors[0].message.contains("BOOLEAN"));
    assert!(errors[0].message.contains("INTEGER"));
}

#[test]
fn rejects_unit_declarations_under_the_default_iso7185_dialect() {
    let source = r#"
        UNIT MathUtils;
        INTERFACE
        IMPLEMENTATION
        END.
    "#;

    let result = compile(source, &CompileOptions::default());

    assert!(result.unit.is_some());
    assert!(result.program.is_none());
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error && d.message.contains("UCSD")));
}

#[test]
fn accepts_unit_declarations_when_ucsd_dialect_is_requested() {
    let source = r#"
        UNIT MathUtils;
        INTERFACE
        IMPLEMENTATION
        END.
    "#;

    let options = CompileOptions::new(Dialect::Ucsd);
    let result = compile(source, &options);

    assert!(result.unit.is_some());
    assert!(
        result.diagnostics.is_empty(),
        "expected no diagnostics under UCSD dialect, got {:?}",
        result.diagnostics
    );
}

#[test]
fn recursive_functions_and_local_procedures_type_check_cleanly() {
    let source = r#"
        PROGRAM Procedures;
        VAR
            result: INTEGER;

        FUNCTION Factorial(n: INTEGER): INTEGER;
        BEGIN
            IF n <= 1 THEN
                Factorial := 1
            ELSE
                Factorial := n * Factorial(n - 1)
        END;

        BEGIN
            result := Factorial(5);
            WriteLn(result)
        END.
    "#;

    let result = compile(source, &CompileOptions::default());

    assert!(result.program.is_some());
    assert!(
        result.diagnostics.is_empty(),
        "expected no diagnostics, got {:?}",
        result.diagnostics
    );
}
