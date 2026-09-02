//! `pmachine-core::PMachine`の統合テスト。`wasd-pcode`が生成したp-codeを
//! 実際に実行し、期待した結果になることを確認する
//! （`crates/pmachine-core`実装指示の「テスト方針」1, 2, 4, 5, 6, 7, 8）。
//!
//! ここに含まれるソース例のいくつかは、
//! `crates/wasd-pcode/tests/codegen.rs`（コード生成そのものを検証する
//! テスト）で使われているものと同じか、それに近い（テスト方針8:
//! 「既存の`wasd-pcode`のテストで使われているPascalソース例を、実際に
//! `pmachine-core`で実行し、期待した結果になることを確認する」）。

mod common;

use pmachine_core::{PMachine, RuntimeError};

/// 1. 定数ロード・算術演算: `1 + 2 * 3`が正しく計算されること。
#[test]
fn arithmetic_expression_computes_the_correct_value() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            x := 1 + 2 * 3
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");

    assert_eq!(vm.global(0), Some(7));
    assert!(vm.is_halted());
}

/// 2a. `IF...THEN...ELSE`が正しく分岐すること（両方の枝を確認）。
#[test]
fn if_then_else_branches_correctly() {
    for (initial, expected) in [(5, 1), (-5, 2)] {
        let module = common::compile(&format!(
            r#"
            PROGRAM P;
            VAR x: INTEGER;
            BEGIN
                x := {initial};
                IF x > 0 THEN
                    x := 1
                ELSE
                    x := 2
            END.
            "#
        ));

        let mut vm = PMachine::new(module);
        vm.run().expect("program should run without error");
        assert_eq!(vm.global(0), Some(expected), "initial x = {initial}");
    }
}

/// 2b. `WHILE`ループが条件が偽になるまで正しく繰り返すこと。
#[test]
fn while_loop_counts_down_to_zero() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            x := 5;
            WHILE x > 0 DO
                x := x - 1
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");
    assert_eq!(vm.global(0), Some(0));
}

/// 2c. `FOR`ループが初期化・増分・終了判定を正しく行うこと。
#[test]
fn for_loop_sums_one_to_five() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR i, sum: INTEGER;
        BEGIN
            sum := 0;
            FOR i := 1 TO 5 DO
                sum := sum + i
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");
    assert_eq!(vm.global(1), Some(15), "sum of 1..=5");
    assert_eq!(vm.global(0), Some(6), "i should be 6 after the loop exits");
}

/// 2d. `REPEAT...UNTIL`が「条件が真になるまで」繰り返すこと
/// （`WHILE`とは逆の極性）。
#[test]
fn repeat_until_counts_down_to_zero() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            x := 5;
            REPEAT
                x := x - 1
            UNTIL x = 0
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");
    assert_eq!(vm.global(0), Some(0));
}

/// 4. `VAR`仮引数の参照渡し: 呼び出し先での変更が呼び出し元に反映されること。
#[test]
fn var_parameter_mutation_is_visible_to_the_caller() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        PROCEDURE Inc(VAR n: INTEGER);
        BEGIN
            n := n + 1
        END;
        BEGIN
            x := 5;
            Inc(x)
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");
    assert_eq!(vm.global(0), Some(6));
}

/// `VAR`仮引数がループ内で繰り返し使われても正しく動作すること
/// （複数回の呼び出しを経てもアドレス解決が壊れないことの確認）。
#[test]
fn var_parameter_works_across_repeated_calls() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR x, i: INTEGER;
        PROCEDURE Inc(VAR n: INTEGER);
        BEGIN
            n := n + 1
        END;
        BEGIN
            x := 0;
            FOR i := 1 TO 10 DO
                Inc(x)
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");
    assert_eq!(vm.global(0), Some(10));
}

/// 5. `FUNCTION`の戻り値が正しく呼び出し元に伝わること。
#[test]
fn function_return_value_is_used_by_the_caller() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR result: INTEGER;
        FUNCTION Square(n: INTEGER): INTEGER;
        BEGIN
            Square := n * n
        END;
        BEGIN
            result := Square(5)
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");
    assert_eq!(vm.global(0), Some(25));
}

/// 6. 再帰呼び出しが正しく動作すること（階乗計算）。
#[test]
fn recursive_function_computes_factorial() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR result: INTEGER;
        FUNCTION Fact(n: INTEGER): INTEGER;
        BEGIN
            IF n <= 1 THEN
                Fact := 1
            ELSE
                Fact := n * Fact(n - 1)
        END;
        BEGIN
            result := Fact(6)
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");
    assert_eq!(vm.global(0), Some(720));
}

/// 6b. 相互に値をやり取りしないが深くネストする再帰でもスタックが
/// 破綻しないこと（フィボナッチ、二重再帰）。
#[test]
fn recursive_function_computes_fibonacci() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR result: INTEGER;
        FUNCTION Fib(n: INTEGER): INTEGER;
        BEGIN
            IF n <= 1 THEN
                Fib := n
            ELSE
                Fib := Fib(n - 1) + Fib(n - 2)
        END;
        BEGIN
            result := Fib(10)
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");
    assert_eq!(vm.global(0), Some(55));
}

/// 7. 0除算で`RuntimeError::DivisionByZero`が返ること（`DIV`）。
#[test]
fn division_by_zero_is_a_runtime_error() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR x, y: INTEGER;
        BEGIN
            y := 0;
            x := 1 DIV y
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    let err = vm
        .run()
        .expect_err("division by zero must be a runtime error");
    assert_eq!(err, RuntimeError::DivisionByZero);
}

/// 7b. `MOD`でも同様に0除算が検出されること。
#[test]
fn modulo_by_zero_is_a_runtime_error() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR x, y: INTEGER;
        BEGIN
            y := 0;
            x := 1 MOD y
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    let err = vm
        .run()
        .expect_err("modulo by zero must be a runtime error");
    assert_eq!(err, RuntimeError::DivisionByZero);
}

/// ネストした制御構造（`IF`の中に`WHILE`）でも正しく実行できること
/// （`crates/wasd-pcode/tests/codegen.rs`の
/// `nested_if_inside_while_resolves_labels_independently`と同じソース）。
#[test]
fn nested_control_structures_execute_correctly() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            x := 3;
            IF x > 0 THEN
                WHILE x > 0 DO
                    x := x - 1
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");
    assert_eq!(vm.global(0), Some(0));
}

/// Step 14, テスト方針1: `WriteLn(整数式)`が正しい値を出力すること。
#[test]
fn writeln_prints_an_integer_value() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            x := 42;
            WriteLn(x)
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "42\n");
}

/// テスト方針2: `WriteLn(Boolean式)`が`true`/`false`を出力すること。
#[test]
fn writeln_prints_a_boolean_value() {
    for (flag, expected) in [("TRUE", "true\n"), ("FALSE", "false\n")] {
        let module = common::compile(&format!(
            r#"
            PROGRAM P;
            VAR flag: BOOLEAN;
            BEGIN
                flag := {flag};
                WriteLn(flag)
            END.
            "#
        ));

        let output = common::CapturedOutput::new();
        let mut vm = PMachine::with_output(module, Box::new(output.clone()));
        vm.run().expect("program should run without error");
        assert_eq!(output.as_string(), expected, "flag = {flag}");
    }
}

/// テスト方針3: 引数なしの`WriteLn`が改行のみ出力すること。
#[test]
fn writeln_with_no_arguments_prints_only_a_newline() {
    let module = common::compile(
        r#"
        PROGRAM P;
        BEGIN
            WriteLn
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "\n");
}

/// テスト方針4: 複数回の`WriteLn`呼び出しが順序通り出力されること。
#[test]
fn multiple_writeln_calls_print_in_order() {
    let module = common::compile(
        r#"
        PROGRAM P;
        BEGIN
            WriteLn(1);
            WriteLn(2);
            WriteLn(3)
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "1\n2\n3\n");
}

/// テスト方針5: `PROCEDURE`内から`WriteLn`を呼び出しても正しく動作し、
/// `CXG`が`CPG`/`RPU`と混在しても活性化レコードの整合性が保たれること。
#[test]
fn writeln_works_from_inside_a_procedure_body() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR result: INTEGER;

        PROCEDURE ShowDouble(n: INTEGER);
        VAR doubled: INTEGER;
        BEGIN
            doubled := n * 2;
            WriteLn(doubled)
        END;

        BEGIN
            result := 21;
            ShowDouble(result);
            WriteLn(result)
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "42\n21\n");
    assert_eq!(vm.global(0), Some(21));
}
