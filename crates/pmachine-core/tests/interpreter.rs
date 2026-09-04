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

/// Step 15, テスト方針1: `WriteLn('Hello, world!')`が正しく文字列を
/// 出力すること（`examples/hello.pas`が実際に動くことの単体テスト版）。
#[test]
fn writeln_prints_a_string_literal() {
    let module = common::compile(
        r#"
        PROGRAM P;
        BEGIN
            WriteLn('Hello, world!')
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "Hello, world!\n");
}

/// Step 15, テスト方針2: 複数の異なる文字列リテラルを含むプログラムで、
/// それぞれ正しい文字列が出力されること（文字列プールのインデックスが
/// 正しく対応していることの確認）。
#[test]
fn multiple_distinct_string_literals_print_in_order() {
    let module = common::compile(
        r#"
        PROGRAM P;
        BEGIN
            WriteLn('first');
            WriteLn('second');
            WriteLn('third')
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "first\nsecond\nthird\n");
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

/// Step 16のゴール: `STRING[n]`変数に文字列リテラルを代入し、`WriteLn`で
/// 出力するプログラム（タスク依頼の動作確認用サンプルそのもの）が正しく
/// 動くこと。
#[test]
fn string_n_variable_assignment_and_writeln_prints_the_assigned_value() {
    let module = common::compile(
        r#"
        PROGRAM StringTest;
        VAR
            greeting: STRING[80];
        BEGIN
            greeting := 'Hello, world!';
            WriteLn(greeting)
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "Hello, world!\n");

    // Word 0 is the string's length prefix; words 1.. hold one character
    // code per word (crate::builtin::BUILTIN_WRITELN_STRVAR documentation,
    // "memory layout" section).
    assert_eq!(vm.global(0), Some(13));
    assert_eq!(vm.global(1), Some('H' as i16));
}

/// Step 16: 複数の`STRING[n]`変数への再代入・出力が、それぞれ独立した
/// 領域に格納され、互いに干渉しないこと。
#[test]
fn multiple_string_n_variables_do_not_interfere_with_each_other() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR
            a: STRING[5];
            b: STRING[10];
        BEGIN
            a := 'Hi';
            b := 'World';
            WriteLn(a);
            WriteLn(b)
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "Hi\nWorld\n");
}

// ---- Step 18: FUNCTION + 引数（値渡し、単一引数）----

/// Step 18のゴール: タスク依頼の動作確認用サンプルそのもの。`FUNCTION`が
/// `INTEGER`の値仮引数を受け取り戻り値を返すこと（`Double`）、`PROCEDURE`が
/// `STRING[n]`の値仮引数を受け取ること（`PrintGreeting`）の両方が同じ
/// プログラム内で正しく動くこと。
#[test]
fn function_and_procedure_with_value_parameters_sample_program_runs_correctly() {
    let module = common::compile(
        r#"
        PROGRAM FuncTest;

        FUNCTION Double(x: INTEGER): INTEGER;
        BEGIN
            Double := x * 2;
        END;

        PROCEDURE PrintGreeting(name: STRING[40]);
        BEGIN
            WriteLn(name);
        END;

        VAR
            n: INTEGER;
        BEGIN
            n := Double(21);
            WriteLn(n);
            PrintGreeting('Hello from a parameter!');
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "42\nHello from a parameter!\n");
    assert!(vm.is_halted());
}

/// Step 18: `STRING[n]`の値仮引数に、リテラルではなく別の`STRING[n]`変数を
/// 渡した場合でも、呼び出し元の変数の中身が一時領域へコピーされ、正しく
/// 出力されること（[`wasd_pcode`]の`CodeGenerator::gen_string_value_arg`
/// が発行する`emit_string_copy_words`の実行結果を検証する）。
#[test]
fn string_n_value_parameter_accepts_a_variable_argument() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR
            greeting: STRING[20];

        PROCEDURE Announce(msg: STRING[20]);
        BEGIN
            WriteLn(msg)
        END;

        BEGIN
            greeting := 'Hi there';
            Announce(greeting)
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "Hi there\n");
}

/// Step 18: `PROCEDURE`/`FUNCTION`の値仮引数として`STRING[n]`を複数回
/// 異なる実引数で呼び出しても、それぞれ独立した一時領域へコピーされ、
/// 互いに干渉しないこと（`CodeGenerator::gen_string_value_arg`が呼び出し
/// ごとに新しい一時領域を`alloc_words`で確保する設計の検証）。
#[test]
fn string_n_value_parameter_calls_do_not_interfere_across_multiple_calls() {
    let module = common::compile(
        r#"
        PROGRAM P;
        PROCEDURE Announce(msg: STRING[10]);
        BEGIN
            WriteLn(msg)
        END;

        BEGIN
            Announce('First');
            Announce('Second')
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "First\nSecond\n");
    assert!(vm.is_halted());
}

// ---- Step 19: 配列（グローバル・1次元・INTEGER/BOOLEAN要素のみ）----
//
// `wasd-pcode`側（`CodeGenerator::gen_array_element_address`のドキュメント
// 「設計判断」参照）が配列添字アクセスを新規のp-codeオペコードを追加せず
// 既存の`LDA`/`LDC`/`SBI`/`ADI`/`IND`/`STI`の組み合わせだけで合成する設計を
// 採ったため、本クレート（`pmachine-core`）自体にはオペコード追加が一切
// 不要だった。以下のテストは、その組み合わせが実際に正しく動作すること
// （`crates/wasd-pcode/tests/codegen.rs`が検証する命令列を、実際に
// `PMachine`で実行して結果を確認する。`crates/pmachine-core`実装指示の
// 「テスト方針8」参照）を確認する。

/// 配列の1要素への書き込みと読み込みが正しく往復すること
/// （`arr[i] := value; x := arr[i]`）。
#[test]
fn array_element_write_then_read_round_trips() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR arr: ARRAY [1..10] OF INTEGER;
        VAR x: INTEGER;
        BEGIN
            arr[3] := 123;
            x := arr[3]
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");

    // arr occupies globals 0..10, x is global 10.
    assert_eq!(vm.global(2), Some(123)); // arr[3] (low=1, so index 2 in the backing store)
    assert_eq!(vm.global(10), Some(123));
    assert!(vm.is_halted());
}

/// `FOR`ループで配列全要素へ書き込み、別の`FOR`ループで合計を計算する
/// （配列 + 制御構造 + 変数間の組み合わせの実行確認）。`arr[i] := i * i`
/// (1から5までの二乗) の合計は `1+4+9+16+25 = 55`。
#[test]
fn for_loop_fills_array_and_a_second_loop_sums_it() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR arr: ARRAY [1..5] OF INTEGER;
        VAR i, total: INTEGER;
        BEGIN
            FOR i := 1 TO 5 DO
                arr[i] := i * i;
            total := 0;
            FOR i := 1 TO 5 DO
                total := total + arr[i]
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");

    // arr: globals 0..5, i: global 5, total: global 6.
    assert_eq!(vm.globals()[0..5], [1, 4, 9, 16, 25]);
    assert_eq!(vm.global(6), Some(55));
    assert!(vm.is_halted());
}

/// 下限が1以外（`ARRAY [5..7]`）の配列でも、宣言された下限を基準に
/// 正しくアドレッシングされること（`CodeGenerator::gen_array_element_address`
/// が`LDC`に積む値は常に宣言された`low`そのものであり、0起点への
/// 正規化を行わない設計であることの実行確認）。
#[test]
fn array_with_non_default_low_bound_indexes_correctly() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR arr: ARRAY [5..7] OF INTEGER;
        BEGIN
            arr[5] := 50;
            arr[6] := 60;
            arr[7] := 70
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    vm.run().expect("program should run without error");

    assert_eq!(vm.globals()[0..3], [50, 60, 70]);
    assert!(vm.is_halted());
}

/// `BOOLEAN`要素の配列も正しく読み書きでき、`WriteLn`で出力できること。
#[test]
fn boolean_array_element_write_then_writeln() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR flags: ARRAY [1..3] OF BOOLEAN;
        BEGIN
            flags[1] := TRUE;
            flags[2] := FALSE;
            flags[3] := TRUE;
            WriteLn(flags[1]);
            WriteLn(flags[2]);
            WriteLn(flags[3])
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "true\nfalse\ntrue\n");
    assert!(vm.is_halted());
}

/// 添字が変数式（定数だけでなく、実行時に計算される値）でも正しく
/// アドレッシングできること（`wasd-sema`のコンパイル時定数添字チェックは
/// リテラル添字のみが対象であり、変数添字は実行時まで検証されない。
/// `crates/wasd-sema/src/typeck.rs`の`infer_index_access_type`ドキュメント
/// 参照）。ここでは配列を逆順に読み出す形で確認する。
#[test]
fn array_index_can_be_a_runtime_computed_expression() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR arr: ARRAY [1..5] OF INTEGER;
        VAR i, j: INTEGER;
        BEGIN
            FOR i := 1 TO 5 DO
                arr[i] := i * 10;
            j := 6;
            FOR i := 1 TO 5 DO
            BEGIN
                j := j - 1;
                WriteLn(arr[j])
            END
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "50\n40\n30\n20\n10\n");
    assert!(vm.is_halted());
}

// ---- Step 20: レコード（フィールドアクセス） ----

/// タスク依頼の動作確認用サンプルプログラムそのもの
/// （Wizardry的なキャラクターデータ構造の最小版）。`hero.hp := 100;`
/// `hero.alive := TRUE;`のフィールド書き込み、`WriteLn(hero.hp)`の
/// フィールド読み出しに加えて、`hero.hp := hero.hp - 30;`という
/// 「同じフィールドを読み出してから書き戻す」パターンも確認する。
#[test]
fn record_field_access_sample_program_runs_correctly() {
    let module = common::compile(
        r#"
        PROGRAM RecordTest;
        TYPE
            Character = RECORD
                hp: INTEGER;
                alive: BOOLEAN;
            END;
        VAR
            hero: Character;
        BEGIN
            hero.hp := 100;
            hero.alive := TRUE;

            WriteLn(hero.hp);

            hero.hp := hero.hp - 30;
            WriteLn(hero.hp)
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "100\n70\n");
    assert!(vm.is_halted());
}

/// レコードのフィールドの読み書きが、配列変数と共存しても互いに干渉
/// しないこと（Step 19の配列がグローバルデータ領域を使うのと同じ領域を
/// レコードも使うため、アドレス割り当てが重ならないことを確認する）。
#[test]
fn record_fields_and_arrays_coexist_without_interfering() {
    let module = common::compile(
        r#"
        PROGRAM P;
        TYPE
            Character = RECORD
                hp: INTEGER;
                alive: BOOLEAN;
            END;
        VAR
            scores: ARRAY [1..3] OF INTEGER;
            hero: Character;
            i: INTEGER;
        BEGIN
            FOR i := 1 TO 3 DO
                scores[i] := i * 100;

            hero.hp := 50;
            hero.alive := TRUE;

            FOR i := 1 TO 3 DO
                WriteLn(scores[i]);
            WriteLn(hero.hp);
            WriteLn(hero.alive)
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "100\n200\n300\n50\ntrue\n");
    assert!(vm.is_halted());
}

/// `TYPE`宣言を経ない無名`RECORD`型の`VAR`宣言でも、フィールドの読み書きが
/// 正しく動作すること。
#[test]
fn anonymous_record_field_access_round_trips() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR point: RECORD x, y: INTEGER END;
        BEGIN
            point.x := 3;
            point.y := 4;
            WriteLn(point.x);
            WriteLn(point.y)
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "3\n4\n");
    assert!(vm.is_halted());
}

/// `STRING[n]`フィールドへの代入・`WriteLn`も実際に実行して正しい文字列が
/// 出力されること（`crates/wasd-pcode/tests/codegen.rs`の
/// `string_n_record_field_assignment_and_writeln_are_supported`が命令列を
/// 検証しているのに対し、こちらは実行結果そのものを検証する）。
#[test]
fn string_n_record_field_assignment_and_writeln_prints_the_assigned_value() {
    let module = common::compile(
        r#"
        PROGRAM P;
        TYPE
            Character = RECORD
                hp: INTEGER;
                name: STRING[20];
            END;
        VAR hero: Character;
        BEGIN
            hero.hp := 10;
            hero.name := 'Gandalf';
            WriteLn(hero.name);
            WriteLn(hero.hp)
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "Gandalf\n10\n");
    assert!(vm.is_halted());
}

// ---- Step 21: 複数引数（任意数）、配列・レコードの値仮引数 ----

/// タスク依頼の動作確認用サンプルプログラムそのもの:
/// - `FUNCTION Add(a, b, c: INTEGER): INTEGER`（3個の`INTEGER`仮引数）
/// - `PROCEDURE Damage(VAR ch: Character; amount: INTEGER)`（レコード型の
///   `VAR`仮引数 + `INTEGER`の値仮引数の混在）
///
/// `VAR`仮引数の構文（`VAR ch: Character`）は本実装で既にサポート済み
/// （`wasd-parser`の`parses_procedure_decl_with_var_param`、
/// `wasd-sema`の`by_ref`チェック参照）のため、タスク依頼が許容する
/// 「VAR構文が未対応なら値渡し引数に書き換えてよい」という代替は不要と
/// 判断し、タスク依頼のソースをそのまま使う。`10 + 20 + 30 = 60`、
/// `50 - 30 = 20`（`<= 0`ではないので`alive`は`TRUE`のまま）で、
/// 期待される出力は完了条件に明記された`"60\n20\n"`と一致する。
#[test]
fn multi_arg_task_sample_program_runs_correctly() {
    let module = common::compile(
        r#"
        PROGRAM MultiArgTest;
        TYPE
            Character = RECORD
                hp: INTEGER;
                alive: BOOLEAN;
            END;

        FUNCTION Add(a: INTEGER; b: INTEGER; c: INTEGER): INTEGER;
        BEGIN
            Add := a + b + c;
        END;

        PROCEDURE Damage(VAR ch: Character; amount: INTEGER);
        BEGIN
            ch.hp := ch.hp - amount;
            IF ch.hp <= 0 THEN
                ch.alive := FALSE;
        END;

        VAR
            hero: Character;
            total: INTEGER;
        BEGIN
            total := Add(10, 20, 30);
            WriteLn(total);

            hero.hp := 50;
            hero.alive := TRUE;
            Damage(hero, 30);
            WriteLn(hero.hp);
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "60\n20\n");
    assert!(vm.is_halted());
}

/// タスク依頼「3つ以上の異なる型を混在させた引数（INTEGER、BOOLEAN、
/// STRING[n]、配列）を持つ呼び出し」パターン。同時に、配列の値仮引数が
/// 仮実装（コピーを作らずアドレスをそのまま渡す。
/// `wasd_pcode::CodeGenerator::gen_array_or_record_value_arg`のドキュメント
/// 「UNCONFIRMED/TODO」参照）であることの既知の副作用——呼び出された側での
/// 配列要素への変更が呼び出し元にも反映されてしまうこと——も実行結果で
/// 確認する（`nums[1]`が`Describe`呼び出し後に呼び出し元から見ても`999`に
/// 変わっていること）。
#[test]
fn procedure_call_with_integer_boolean_string_and_array_parameters_mixed_runs_correctly() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR
            nums: ARRAY [1..3] OF INTEGER;

        PROCEDURE Describe(n: INTEGER; flag: BOOLEAN; msg: STRING[10]; arr: ARRAY [1..3] OF INTEGER);
        BEGIN
            WriteLn(n);
            WriteLn(flag);
            WriteLn(msg);
            WriteLn(arr[1]);
            WriteLn(arr[2]);
            WriteLn(arr[3]);
            arr[1] := 999;
        END;

        BEGIN
            nums[1] := 1;
            nums[2] := 2;
            nums[3] := 3;
            Describe(42, TRUE, 'hi', nums);
            WriteLn(nums[1])
        END.
        "#,
    );

    let output = common::CapturedOutput::new();
    let mut vm = PMachine::with_output(module, Box::new(output.clone()));
    vm.run().expect("program should run without error");
    assert_eq!(output.as_string(), "42\ntrue\nhi\n1\n2\n3\n999\n");
    assert!(vm.is_halted());
}
