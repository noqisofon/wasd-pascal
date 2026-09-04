//! `wasd-pcode::CodeGenerator`の統合テスト。
//!
//! レキサ→パーサーを通してASTを組み立て（意味解析は本テストの関心事では
//! ないため通さない）、`CodeGenerator::generate`が生成するp-code命令列を
//! 検証する。命令のニーモニック自体は`crates/wasd-pcode/src/opcode.rs`の
//! `UnconfirmedOp`のドキュメントに記載の通り、このセッションでは一次資料
//! （SofTech Microsystems, *UCSD p-System and UCSD Pascal Version IV:
//! Internal Architecture Guide*; T. Nouspikel's TI-99/4A p-System実装
//! ガイド）に一切あたれなかったため**未確認（UNCONFIRMED）**である。
//! 期待値はいずれも`UnconfirmedOp`のバリアントそのもの（テスト対象の実装が
//! 使っているのと同じ表現）で記述しており、「一次資料で確認済みの
//! オペコード番号」を検証しているわけではない点に注意。

use wasd_ast::Program;
use wasd_lexer::Lexer;
use wasd_parser::Parser;
use wasd_pcode::{
    Address, CodeAddress, CodeGenerator, ConfirmedOp, Level, Opcode, PCodeModule, UnconfirmedOp,
    BUILTIN_WRITELN_BOOL, BUILTIN_WRITELN_INT, BUILTIN_WRITELN_NONE, BUILTIN_WRITELN_STRING,
    BUILTIN_WRITELN_STRVAR, KERNEL_SEGMENT,
};

fn parse_program(source: &str) -> Program {
    let mut lexer = Lexer::new(source);
    let (tokens, lex_diags) = lexer.tokenize();
    assert!(lex_diags.is_empty(), "lexer diagnostics: {lex_diags:?}");

    let mut parser = Parser::new(tokens);
    let (program, parse_diags) = parser.parse_program();
    assert!(
        parse_diags.is_empty(),
        "parser diagnostics: {parse_diags:?}"
    );
    program.expect("source should parse into a Program")
}

fn opcodes(module: &PCodeModule) -> Vec<Opcode> {
    module.instructions.iter().map(|i| i.opcode).collect()
}

fn op(o: UnconfirmedOp) -> Opcode {
    Opcode::Unconfirmed(o)
}

fn cop(o: ConfirmedOp) -> Opcode {
    Opcode::Confirmed(o)
}

/// 1. 単純な代入文 `x := 1 + 2` のp-code生成が期待通りの命令列になること。
#[test]
fn simple_assignment_generates_expected_instructions() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            x := 1 + 2
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Ldc(2)),
            op(UnconfirmedOp::Adi),
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Stp),
        ]
    );
    assert_eq!(module.global_data_words, 1);
}

/// 2. `IF...THEN...ELSE`の分岐命令とラベルが正しく生成されること。
#[test]
fn if_then_else_generates_fjp_and_ujp_with_correctly_patched_targets() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            IF x > 0 THEN
                x := 1
            ELSE
                x := 2
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Lod(Level(0), Address(0))), // x
            op(UnconfirmedOp::Ldc(0)),
            // `x > 0` = `NOT (x <= 0)` (LEQI + NOT; `>` doesn't exist as an
            // opcode, `crate::codegen::CodeGenerator::emit_binop`のドキュメント参照)
            op(UnconfirmedOp::Leq),
            op(UnconfirmedOp::Not),
            op(UnconfirmedOp::Fjp(CodeAddress(8))), // -> else branch (index 8)
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Ujp(CodeAddress(10))), // -> past else branch (STP)
            op(UnconfirmedOp::Ldc(2)),
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// `IF...THEN`（`ELSE`なし）では余計な`UJP`が発行されず、`FJP`は
/// `THEN`本体の直後（ここでは`STP`）を指すこと。
#[test]
fn if_then_without_else_only_emits_a_single_forward_jump() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            IF x > 0 THEN
                x := 1
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Lod(Level(0), Address(0))),
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Leq), // `x > 0` = NOT (x <= 0)
            op(UnconfirmedOp::Not),
            op(UnconfirmedOp::Fjp(CodeAddress(7))), // -> STP, right after THEN body
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// 3. `WHILE`ループの分岐（ループ先頭への戻り、条件不成立での脱出）が
///    正しく生成されること。
#[test]
fn while_loop_generates_backward_and_forward_jumps() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            WHILE x > 0 DO
                x := x - 1
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Lod(Level(0), Address(0))), // loop start (index 0)
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Leq), // `x > 0` = NOT (x <= 0)
            op(UnconfirmedOp::Not),
            op(UnconfirmedOp::Fjp(CodeAddress(10))), // -> STP, past the loop
            op(UnconfirmedOp::Lod(Level(0), Address(0))),
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Sbi),
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Ujp(CodeAddress(0))), // -> back to loop start
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// 4. `FOR`ループの初期化・増分・終了判定が正しく生成されること。
///
/// 終了値（`end`式）はISO Pascalの規定通りループ開始前に一度だけ評価され、
/// 隠し一時変数（`i`とは別のアドレス）に保持される
/// （`crates/wasd-pcode/src/codegen.rs`の`gen_for`ドキュメント参照）。
#[test]
fn for_loop_generates_init_test_body_and_increment() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR i: INTEGER;
        BEGIN
            FOR i := 1 TO 10 DO
                i := i
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Str(Level(0), Address(0))), // i := 1
            op(UnconfirmedOp::Ldc(10)),
            op(UnconfirmedOp::Str(Level(0), Address(1))), // hidden limit := 10
            op(UnconfirmedOp::Lod(Level(0), Address(0))), // loop start (index 4)
            op(UnconfirmedOp::Lod(Level(0), Address(1))),
            op(UnconfirmedOp::Leq),                       // i <= limit
            op(UnconfirmedOp::Fjp(CodeAddress(15))),      // -> STP, past the loop
            op(UnconfirmedOp::Lod(Level(0), Address(0))), // body: i := i
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Lod(Level(0), Address(0))),
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Adi), // i + 1
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Ujp(CodeAddress(4))), // -> back to loop start
            op(UnconfirmedOp::Stp),
        ]
    );
    assert_eq!(module.global_data_words, 2, "i and the hidden limit temp");
}

/// `DOWNTO`では終了判定に`GEQ`、増分に`SBI`が使われること。
#[test]
fn for_downto_uses_geq_and_sbi() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR i: INTEGER;
        BEGIN
            FOR i := 10 DOWNTO 1 DO
                i := i
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");
    let ops = opcodes(&module);

    assert!(ops.contains(&op(UnconfirmedOp::Geq)));
    assert!(ops.contains(&op(UnconfirmedOp::Sbi)));
    assert!(!ops.contains(&op(UnconfirmedOp::Leq)));
    assert!(!ops.contains(&op(UnconfirmedOp::Adi)));
}

/// 5. ネストした制御構造（`IF`の中に`WHILE`）でラベル解決が壊れないこと。
#[test]
fn nested_if_inside_while_resolves_labels_independently() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            IF x > 0 THEN
                WHILE x > 0 DO
                    x := x - 1
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Lod(Level(0), Address(0))), // outer IF condition (index 0)
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Leq), // `x > 0` = NOT (x <= 0)
            op(UnconfirmedOp::Not),
            op(UnconfirmedOp::Fjp(CodeAddress(15))), // outer IF -> STP
            op(UnconfirmedOp::Lod(Level(0), Address(0))), // inner WHILE condition (loop start, index 5)
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Leq), // `x > 0` = NOT (x <= 0)
            op(UnconfirmedOp::Not),
            op(UnconfirmedOp::Fjp(CodeAddress(15))), // inner WHILE -> STP
            op(UnconfirmedOp::Lod(Level(0), Address(0))),
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Sbi),
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Ujp(CodeAddress(5))), // inner WHILE -> back to loop start
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// `REPEAT ... UNTIL`は後方分岐のみで実装され、`FJP`が偽の間ループ先頭へ
/// 戻ること（真になったら抜ける、`WHILE`とは逆の極性）。
#[test]
fn repeat_until_generates_a_single_backward_conditional_jump() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            REPEAT
                x := x - 1
            UNTIL x = 0
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Lod(Level(0), Address(0))), // loop start (index 0): body
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Sbi),
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Lod(Level(0), Address(0))), // UNTIL x = 0
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Equ),
            op(UnconfirmedOp::Fjp(CodeAddress(0))), // false -> back to loop start
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// 6. `PROCEDURE`/`FUNCTION`呼び出しのp-code生成（Step 12）。
///
/// 引数なしの単純な呼び出し。`Foo`（`PROGRAM`直下、lexレベル1）は
/// メイン本体より前に生成されるため、`CPG`の呼び出し先アドレスは
/// バックパッチ不要で直接確定できる。
#[test]
fn no_arg_procedure_call_generates_cpg_and_rpu() {
    let program = parse_program(
        r#"
        PROGRAM P;
        PROCEDURE Foo;
        BEGIN
        END;
        BEGIN
            Foo
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            cop(ConfirmedOp::Rpu(0)),              // Foo's body (entry = 0)
            cop(ConfirmedOp::Cpg(CodeAddress(0))), // Foo() call
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// 値引数を持つ`PROCEDURE`呼び出し: 呼び出し元は式を評価した値を積み、
/// 呼び出し先はそれをローカルスコープ（レベル差0）の仮引数スロットとして
/// 読む。
#[test]
fn procedure_call_with_value_parameter_pushes_the_evaluated_value() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR result: INTEGER;
        PROCEDURE Double(n: INTEGER);
        BEGIN
            result := n + n
        END;
        BEGIN
            Double(21)
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            // Double's body (entry = 0): result := n + n
            op(UnconfirmedOp::Lod(Level(0), Address(5))), // n (own frame, level 0)
            op(UnconfirmedOp::Lod(Level(0), Address(5))), // n
            op(UnconfirmedOp::Adi),
            op(UnconfirmedOp::Str(Level(1), Address(0))), // result (global, level 1)
            cop(ConfirmedOp::Rpu(1)),
            // Double(21) call
            op(UnconfirmedOp::Ldc(21)),
            cop(ConfirmedOp::Cpg(CodeAddress(0))),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// `VAR`引数を持つ`PROCEDURE`呼び出し: 呼び出し元は値ではなく
/// **アドレス**（`LDA`）を積み、呼び出し先はそのスロットを`LOD`+`IND`/
/// `LOD`+(値を積んで)`STI`で間接的に読み書きする。
#[test]
fn procedure_call_with_var_parameter_pushes_the_address_of_the_argument() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        PROCEDURE Inc(VAR n: INTEGER);
        BEGIN
            n := n + 1
        END;
        BEGIN
            Inc(x)
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            // Inc's body (entry = 0): n := n + 1 (n is a VAR parameter)
            op(UnconfirmedOp::Lod(Level(0), Address(5))), // n's slot holds an address
            op(UnconfirmedOp::Lod(Level(0), Address(5))), // ... read it again to dereference
            op(UnconfirmedOp::Ind),                       // ... and load the referenced value
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Adi),
            op(UnconfirmedOp::Sti), // store through the address pushed first
            cop(ConfirmedOp::Rpu(1)),
            // Inc(x) call: the caller pushes the ADDRESS of x, not its value
            op(UnconfirmedOp::Lda(Level(0), Address(0))),
            cop(ConfirmedOp::Cpg(CodeAddress(0))),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// `FUNCTION`の戻り値が正しく設定され、呼び出し元へ返ること。
/// `FunctionName := value`は戻り値スロット（活性化レコード末尾）への
/// `STR`として、呼び出し元での使用は`CPG`直後にその1ワードがスタックに
/// 残っているものとして扱われる。
#[test]
fn function_call_stores_and_returns_its_return_value() {
    let program = parse_program(
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

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            // Square's body (entry = 0): Square := n * n
            op(UnconfirmedOp::Lod(Level(0), Address(5))), // n
            op(UnconfirmedOp::Lod(Level(0), Address(5))), // n
            op(UnconfirmedOp::Mpi),
            op(UnconfirmedOp::Str(Level(0), Address(6))), // return value slot (5 + data_size(0) + 1 param)
            cop(ConfirmedOp::Rpu(1)),
            // result := Square(5)
            op(UnconfirmedOp::Ldc(5)),
            cop(ConfirmedOp::Cpg(CodeAddress(0))),
            op(UnconfirmedOp::Str(Level(0), Address(0))), // result
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// 再帰呼び出し: 自分自身をまだ本体生成中に呼ぶ場合でも、
/// [`CodeGenerator::begin_routine_body`]が本体生成の直前にエントリ
/// アドレスを確定させるため、バックパッチなしで正しい`CPG`が発行される
/// こと。
#[test]
fn recursive_function_call_uses_cpg_targeting_its_own_entry() {
    let program = parse_program(
        r#"
        PROGRAM P;
        FUNCTION Fact(n: INTEGER): INTEGER;
        BEGIN
            IF n <= 1 THEN
                Fact := 1
            ELSE
                Fact := n * Fact(n - 1)
        END;
        BEGIN
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");
    let ops = opcodes(&module);

    // Fact自身の本体はentry = CodeAddress(0)から始まる。再帰呼び出しは
    // その0を指すCPGとして現れるはず。
    assert!(
        ops.contains(&cop(ConfirmedOp::Cpg(CodeAddress(0)))),
        "expected a recursive CPG(0) call, got {ops:?}"
    );
    // IF/ELSEの分岐（FJP/UJP）がバックパッチされ、末尾はRPU/STPで終わる。
    assert!(ops
        .iter()
        .any(|o| matches!(o, Opcode::Unconfirmed(UnconfirmedOp::Fjp(_)))));
    assert!(ops
        .iter()
        .any(|o| matches!(o, Opcode::Unconfirmed(UnconfirmedOp::Ujp(_)))));
    assert!(matches!(
        ops[ops.len() - 2],
        Opcode::Confirmed(ConfirmedOp::Rpu(_))
    ));
    assert_eq!(ops[ops.len() - 1], op(UnconfirmedOp::Stp));
}

/// ネストしたスコープでの変数参照: `PROCEDURE`本体からローカル変数を
/// 読む場合はレベル差0、外側（`PROGRAM`）のグローバル変数を読む場合は
/// レベル差1になること。
#[test]
fn local_and_global_variable_references_use_different_levels() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR g: INTEGER;
        PROCEDURE UseBoth(n: INTEGER);
        VAR local: INTEGER;
        BEGIN
            local := n;
            g := local
        END;
        BEGIN
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Lod(Level(0), Address(6))), // n (own frame)
            op(UnconfirmedOp::Str(Level(0), Address(5))), // local := n (own frame)
            op(UnconfirmedOp::Lod(Level(0), Address(5))), // local (own frame)
            op(UnconfirmedOp::Str(Level(1), Address(0))), // g := local (one level up: global)
            cop(ConfirmedOp::Rpu(2)),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// 今回のスコープ外の`PROCEDURE`/`FUNCTION`構文（配列型の仮引数）を
/// 含むASTを渡した場合、パニックせず明確なエラーが返ること。
#[test]
fn procedure_with_unsupported_parameter_type_is_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        PROCEDURE Foo(arr: ARRAY [1..10] OF INTEGER);
        BEGIN
        END;
        BEGIN
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics =
        result.expect_err("unsupported parameter types must be rejected, not codegen'd");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("ARRAY")),
        "diagnostics: {diagnostics:?}"
    );
}

/// `CASE`文も同様にスコープ外としてエラー報告されること（パニックしない）。
#[test]
fn case_statements_are_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            CASE x OF
                1: x := 1;
                2: x := 2
            END
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("CASE statements must be rejected, not codegen'd");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("CASE")),
        "diagnostics: {diagnostics:?}"
    );
}

/// レコード・ポインタ型の`VAR`宣言はスコープ外としてエラー報告される
/// こと（配列型自体はStep 19からグローバル変数として扱えるようになった。
/// 下記「Step 19: 配列」節参照）。
#[test]
fn record_typed_variables_are_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR r: RECORD x: INTEGER END;
        BEGIN
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("record VARs must be rejected, not codegen'd");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("RECORD")),
        "diagnostics: {diagnostics:?}"
    );
}

/// 7. `WriteLn(整数式)`が`CXG <KERNEL_SEGMENT>, <BUILTIN_WRITELN_INT>`を
///    発行すること（Step 14）。
#[test]
fn writeln_with_integer_argument_emits_cxg_writeln_int() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        BEGIN
            x := 42;
            WriteLn(x)
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Ldc(42)),
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Lod(Level(0), Address(0))),
            cop(ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_INT)),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// `WriteLn(Boolean式)`が`BUILTIN_WRITELN_BOOL`を呼ぶこと。
#[test]
fn writeln_with_boolean_argument_emits_cxg_writeln_bool() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR flag: BOOLEAN;
        BEGIN
            flag := TRUE;
            WriteLn(flag)
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");
    let ops = opcodes(&module);

    assert!(
        ops.contains(&cop(ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_BOOL))),
        "expected a WriteLn(BOOLEAN) call, got {ops:?}"
    );
}

/// 引数なしの`WriteLn`が`BUILTIN_WRITELN_NONE`を呼ぶこと（改行のみ出力）。
#[test]
fn writeln_with_no_arguments_emits_cxg_writeln_none() {
    let program = parse_program(
        r#"
        PROGRAM P;
        BEGIN
            WriteLn
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            cop(ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_NONE)),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// Step 15: `WriteLn('...')`（文字列リテラル）が文字列を`string_pool`へ
/// 追加し、そのインデックスを`LDC`で積んでから`BUILTIN_WRITELN_STRING`を
/// 呼ぶこと（`crates/wasd-pcode/src/builtin.rs`の`BUILTIN_WRITELN_STRING`
/// ドキュメント参照）。
#[test]
fn writeln_with_string_literal_interns_it_and_emits_cxg_writeln_string() {
    let program = parse_program(
        r#"
        PROGRAM P;
        BEGIN
            WriteLn('Hello, world!')
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(module.string_pool, vec!["Hello, world!".to_string()]);
    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Ldc(0)), // index 0 into string_pool
            cop(ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_STRING)),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// 複数の異なる文字列リテラルを含むプログラムで、それぞれ別のインデックス
/// が割り当てられ、`string_pool`の対応する位置に正しく格納されること。
#[test]
fn multiple_distinct_string_literals_get_distinct_pool_indices() {
    let program = parse_program(
        r#"
        PROGRAM P;
        BEGIN
            WriteLn('first');
            WriteLn('second')
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        module.string_pool,
        vec!["first".to_string(), "second".to_string()]
    );
    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Ldc(0)),
            cop(ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_STRING)),
            op(UnconfirmedOp::Ldc(1)),
            cop(ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_STRING)),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// `WriteLn(a, b)`（複数引数）は今回のスコープ外としてエラー報告される
/// こと（パニックしない）。
#[test]
fn writeln_with_multiple_arguments_is_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR a, b: INTEGER;
        BEGIN
            WriteLn(a, b)
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("multi-argument WriteLn must be rejected, not codegen'd");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("WriteLn")),
        "diagnostics: {diagnostics:?}"
    );
}

/// 8. `<`/`>`は一次資料に存在しないため、`GEQI`/`LEQI`+`NOT`で合成される
///    こと（Step 10のUNCONFIRMED解消、Step 14でCONFIRMEDに更新）。
#[test]
fn less_than_and_greater_than_are_synthesized_from_geqi_leqi_and_not() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR a, b, x, y: INTEGER;
        BEGIN
            IF a < b THEN
                x := 1;
            IF a > b THEN
                y := 1
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");
    let ops = opcodes(&module);

    // `a < b` -> GEQI + NOT (in that order, adjacent).
    let geq_not = ops
        .windows(2)
        .any(|w| w == [op(UnconfirmedOp::Geq), op(UnconfirmedOp::Not)]);
    assert!(
        geq_not,
        "expected GEQI immediately followed by NOT for `<`, got {ops:?}"
    );
    // `a > b` -> LEQI + NOT (in that order, adjacent).
    let leq_not = ops
        .windows(2)
        .any(|w| w == [op(UnconfirmedOp::Leq), op(UnconfirmedOp::Not)]);
    assert!(
        leq_not,
        "expected LEQI immediately followed by NOT for `>`, got {ops:?}"
    );
}

// ---- Step 16: STRING[n] ----

/// `STRING[n]`型のグローバル`VAR`が`1 + n`ワード確保すること
/// （`crate::builtin::BUILTIN_WRITELN_STRVAR`のドキュメント「メモリ
/// レイアウト」参照: 先頭1ワードが長さ、続く`n`ワードが文字データ）。
#[test]
fn string_n_variable_reserves_one_plus_max_len_words() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR s: STRING[5];
        BEGIN
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(module.global_data_words, 6);
}

/// `s := 'Hi';`（`s: STRING[5]`）が「長さを`Address(0)`へ`STR`、続けて
/// 各文字コードを`Address(1)`, `Address(2)`, ...へ`STR`」という命令列に
/// なること（`CodeGenerator::gen_string_literal_assignment`参照）。
#[test]
fn string_literal_assignment_generates_length_and_char_stores() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR s: STRING[5];
        BEGIN
            s := 'Hi'
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Ldc(2)), // length
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Ldc('H' as i16)),
            op(UnconfirmedOp::Str(Level(0), Address(1))),
            op(UnconfirmedOp::Ldc('i' as i16)),
            op(UnconfirmedOp::Str(Level(0), Address(2))),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// `WriteLn(s)`（`s`が`STRING[n]`変数）が、値ではなく変数の**アドレス**
/// （`LDA`）を積んでから`BUILTIN_WRITELN_STRVAR`を呼ぶこと
/// （`crate::builtin::BUILTIN_WRITELN_STRVAR`ドキュメント参照）。
#[test]
fn writeln_with_string_n_variable_emits_lda_and_cxg_writeln_strvar() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR s: STRING[80];
        BEGIN
            s := 'Hello, world!';
            WriteLn(s)
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");
    let ops = opcodes(&module);

    assert!(
        ops.contains(&op(UnconfirmedOp::Lda(Level(0), Address(0)))),
        "expected LDA of the string variable's address, got {ops:?}"
    );
    assert!(
        ops.contains(&cop(ConfirmedOp::Cxg(
            KERNEL_SEGMENT,
            BUILTIN_WRITELN_STRVAR
        ))),
        "expected a WriteLn(STRING[n]) call, got {ops:?}"
    );
    // The LDA must immediately precede the CXG call (no value push in between).
    let lda_then_cxg = ops.windows(2).any(|w| {
        w == [
            op(UnconfirmedOp::Lda(Level(0), Address(0))),
            cop(ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_STRVAR)),
        ]
    });
    assert!(
        lda_then_cxg,
        "expected LDA immediately followed by CXG WRITELN_STRVAR, got {ops:?}"
    );
}

/// 宣言された最大長を超える文字列リテラルの代入はスコープ外
/// （コード生成エラー）として報告されること
/// （`CodeGenerator::gen_string_literal_assignment`参照。`wasd-sema`が
/// 通常はこれを型エラーとして先に検出するはずだが、本テストは
/// 意味解析を経由しない`CodeGenerator`単体の防御的チェックを検証する）。
#[test]
fn string_literal_longer_than_max_len_is_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR s: STRING[4];
        BEGIN
            s := 'Hello'
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics =
        result.expect_err("a string literal longer than the declared max length must be rejected");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("STRING[4]")),
        "diagnostics: {diagnostics:?}"
    );
}

/// `STRING[n]`のローカル変数（`PROCEDURE`/`FUNCTION`本体内の`VAR`宣言）は
/// このステップのスコープ外としてエラー報告されること
/// （`CodeGenerator::build_locals`参照。`PROGRAM`直下のグローバル`VAR`のみ
/// サポートする）。
#[test]
fn string_n_local_variable_is_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        PROCEDURE Greet;
        VAR s: STRING[10];
        BEGIN
        END;
        BEGIN
            Greet
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics =
        result.expect_err("STRING[n] local variables must be rejected, not codegen'd");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("STRING[n] local variables")),
        "diagnostics: {diagnostics:?}"
    );
}

// ---- Step 18: FUNCTION + 引数（値渡し、単一引数）----

/// `PROCEDURE`が`STRING[n]`の値仮引数を1つ受け取り、文字列リテラルの
/// 実引数を渡して呼び出す場合の完全な命令列。
///
/// タスク0のUNCONFIRMED判断（`CodeGenerator::gen_string_value_arg`の
/// ドキュメント参照: レコード・配列の値パラメータと同様、パラメータ
/// 領域にはアドレスを格納する）に従い、呼び出し元は:
/// 1. 新規の一時領域（グローバルデータ領域、`Address(0)`から
///    `1 + max_len`ワード）へ文字列リテラルの内容を書き込み、
/// 2. その一時領域自身のアドレスを`LDA`で積んでから`CPG`を発行する。
///
/// 呼び出し先（`Greet`）は仮引数スロット（`Address(5)`、
/// `indirect = true`）から`LOD`だけでそのアドレスを取り出し、
/// `BUILTIN_WRITELN_STRVAR`へそのまま渡す（`VAR`仮引数と同じ
/// アドレッシング。`FrameSlot`のドキュメント参照）。
#[test]
fn procedure_with_string_n_value_parameter_materializes_literal_argument() {
    let program = parse_program(
        r#"
        PROGRAM P;
        PROCEDURE Greet(name: STRING[2]);
        BEGIN
            WriteLn(name)
        END;
        BEGIN
            Greet('Hi')
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            // Greet's body (entry = 0): WriteLn(name)
            op(UnconfirmedOp::Lod(Level(0), Address(5))), // name (stores an address)
            cop(ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_STRVAR)),
            cop(ConfirmedOp::Rpu(1)), // data_size(0) + 1 param word
            // Greet('Hi'): materialize the literal into a fresh temp, then call.
            op(UnconfirmedOp::Ldc(2)), // length of "Hi"
            op(UnconfirmedOp::Str(Level(0), Address(0))),
            op(UnconfirmedOp::Ldc('H' as i16)),
            op(UnconfirmedOp::Str(Level(0), Address(1))),
            op(UnconfirmedOp::Ldc('i' as i16)),
            op(UnconfirmedOp::Str(Level(0), Address(2))),
            op(UnconfirmedOp::Lda(Level(0), Address(0))), // address of the temp
            cop(ConfirmedOp::Cpg(CodeAddress(0))),
            op(UnconfirmedOp::Stp),
        ]
    );
    // The temp (3 words: 1 length + 2 chars) must be counted in the global
    // data area's size even though it's never a user-declared VAR.
    assert_eq!(module.global_data_words, 3);
}

/// `STRING[n]`の値仮引数に、リテラルではなく別の`STRING[n]`変数（直接
/// 記憶方式のグローバル`VAR`）を渡す場合、`emit_string_copy_words`が
/// ワード単位でコピーする命令列を発行すること。
#[test]
fn string_n_value_parameter_accepts_a_direct_variable_argument() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR greeting: STRING[3];
        PROCEDURE Announce(msg: STRING[3]);
        BEGIN
        END;
        BEGIN
            Announce(greeting)
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    // `greeting` occupies Address(0..=3) (1 + 3 words); the fresh temp for
    // the call comes right after it, at Address(4..=7).
    assert_eq!(
        opcodes(&module),
        vec![
            // Announce's body (entry = 0): empty.
            cop(ConfirmedOp::Rpu(1)),
            // Announce(greeting): copy greeting's 4 words into a fresh temp.
            op(UnconfirmedOp::Lod(Level(0), Address(0))),
            op(UnconfirmedOp::Str(Level(0), Address(4))),
            op(UnconfirmedOp::Lod(Level(0), Address(1))),
            op(UnconfirmedOp::Str(Level(0), Address(5))),
            op(UnconfirmedOp::Lod(Level(0), Address(2))),
            op(UnconfirmedOp::Str(Level(0), Address(6))),
            op(UnconfirmedOp::Lod(Level(0), Address(3))),
            op(UnconfirmedOp::Str(Level(0), Address(7))),
            op(UnconfirmedOp::Lda(Level(0), Address(4))),
            cop(ConfirmedOp::Cpg(CodeAddress(0))),
            op(UnconfirmedOp::Stp),
        ]
    );
    assert_eq!(module.global_data_words, 8);
}

/// 既に`STRING[n]`仮引数として受け取った値を、さらに別の呼び出しへ
/// `STRING[n]`値引数として中継することは今回のスコープ外として
/// エラー報告されること（`CodeGenerator::gen_string_value_arg`参照）。
#[test]
fn relaying_a_received_string_n_parameter_as_another_value_argument_is_an_error() {
    let program = parse_program(
        r#"
        PROGRAM P;
        PROCEDURE Inner(s: STRING[5]);
        BEGIN
        END;
        PROCEDURE Outer(s: STRING[5]);
        BEGIN
            Inner(s)
        END;
        BEGIN
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("relaying a received STRING[n] parameter must be rejected");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("out of scope")),
        "diagnostics: {diagnostics:?}"
    );
}

/// `FUNCTION`/`PROCEDURE`の`STRING[n]`型の仮引数自体は、もはやスコープ外
/// ではない（Step 16まではエラーだったが、Step 18でサポートした）ことの
/// 回帰確認。
#[test]
fn string_n_parameter_is_no_longer_out_of_scope() {
    let program = parse_program(
        r#"
        PROGRAM P;
        PROCEDURE Greet(name: STRING[10]);
        BEGIN
        END;
        BEGIN
            Greet('Hi')
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    assert!(
        result.is_ok(),
        "STRING[n] parameters should be supported from Step 18 onward: {result:?}"
    );
}

// ---- Step 19: 配列（グローバル・1次元・INTEGER/BOOLEAN要素のみ） ----

/// `ARRAY [low..high] OF INTEGER`のグローバル`VAR`が`high - low + 1`ワード
/// 確保すること（下限が1でない場合、複数の名前を1つの宣言で共有する場合の
/// 両方を確認）。
#[test]
fn global_array_variable_reserves_high_minus_low_plus_one_words() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR a: ARRAY [1..10] OF INTEGER;
        VAR b: ARRAY [5..7] OF INTEGER;
        VAR c, d: ARRAY [0..2] OF BOOLEAN;
        BEGIN
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    // a: 10 words (0..10), b: 3 words (10..13), c: 3 words (13..16), d: 3 words (16..19)
    assert_eq!(module.global_data_words, 19);
}

/// `x := arr[i]`（`arr`がグローバル配列変数）が、
/// `LDA <arr先頭> ; <i> ; LDC <low> ; SBI ; ADI ; IND ; STR <x>`という
/// 命令列になること（`CodeGenerator::gen_array_element_address`の設計判断
/// ドキュメント参照: 専用オペコードを新設せず、既存の`LDA`/`IND`と算術
/// 命令だけで合成する）。
#[test]
fn array_index_load_generates_lda_ldc_sbi_adi_ind() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR arr: ARRAY [1..10] OF INTEGER;
        VAR i, x: INTEGER;
        BEGIN
            x := arr[i]
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    // arr: Address(0)..Address(10), i: Address(10), x: Address(11)
    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Lda(Level(0), Address(0))),  // &arr[1]
            op(UnconfirmedOp::Lod(Level(0), Address(10))), // i
            op(UnconfirmedOp::Ldc(1)),                     // low
            op(UnconfirmedOp::Sbi),                        // i - low
            op(UnconfirmedOp::Adi),                        // &arr[1] + (i - low)
            op(UnconfirmedOp::Ind),                        // arr[i]
            op(UnconfirmedOp::Str(Level(0), Address(11))), // x := ...
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// `arr[i] := x`が、`LDA <arr先頭> ; <i> ; LDC <low> ; SBI ; ADI ; <x> ; STI`
/// という命令列になること（読み込みと対称に、最後が`IND`ではなく値を積んで
/// からの`STI`になる）。
#[test]
fn array_index_store_generates_lda_ldc_sbi_adi_sti() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR arr: ARRAY [1..10] OF INTEGER;
        VAR i, x: INTEGER;
        BEGIN
            arr[i] := x
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Lda(Level(0), Address(0))),  // &arr[1]
            op(UnconfirmedOp::Lod(Level(0), Address(10))), // i
            op(UnconfirmedOp::Ldc(1)),                     // low
            op(UnconfirmedOp::Sbi),                        // i - low
            op(UnconfirmedOp::Adi),                        // &arr[1] + (i - low)
            op(UnconfirmedOp::Lod(Level(0), Address(11))), // x
            op(UnconfirmedOp::Sti),                        // arr[i] := x
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// 下限が1以外（例: `ARRAY [5..7]`）でも、`LDC`に積む下限がその値
/// そのものになること（0起点への正規化を勝手に行わない）。
#[test]
fn array_index_uses_the_declared_low_bound_as_is() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR arr: ARRAY [5..7] OF INTEGER;
        BEGIN
            arr[5] := 42
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert_eq!(
        opcodes(&module),
        vec![
            op(UnconfirmedOp::Lda(Level(0), Address(0))),
            op(UnconfirmedOp::Ldc(5)), // index literal
            op(UnconfirmedOp::Ldc(5)), // low bound
            op(UnconfirmedOp::Sbi),
            op(UnconfirmedOp::Adi),
            op(UnconfirmedOp::Ldc(42)),
            op(UnconfirmedOp::Sti),
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// `WriteLn(arr[i])`が、通常の`INTEGER`式と同じく`BUILTIN_WRITELN_INT`を
/// 呼ぶこと（`infer_expr_kind`の`IndexAccess`腕が要素の種類を正しく
/// 返すことの確認）。
#[test]
fn writeln_with_array_element_emits_cxg_writeln_int() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR arr: ARRAY [1..3] OF INTEGER;
        BEGIN
            WriteLn(arr[1])
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert!(
        module
            .instructions
            .iter()
            .any(|i| i.opcode == cop(ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_INT))),
        "expected a WriteLn(INTEGER) call: {:?}",
        opcodes(&module)
    );
}

/// `BOOLEAN`要素の配列も同様にサポートされ、`WriteLn`は
/// `BUILTIN_WRITELN_BOOL`を呼ぶこと。
#[test]
fn boolean_array_element_supports_writeln_bool() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR flags: ARRAY [1..3] OF BOOLEAN;
        BEGIN
            flags[1] := TRUE;
            WriteLn(flags[1])
        END.
        "#,
    );

    let module = CodeGenerator::new()
        .generate(&program)
        .expect("codegen should succeed");

    assert!(
        module
            .instructions
            .iter()
            .any(|i| i.opcode == cop(ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_BOOL))),
        "expected a WriteLn(BOOLEAN) call: {:?}",
        opcodes(&module)
    );
}

/// 配列全体を1ワードの値として読み込む（添字を付けない裸の配列参照）ことは
/// 依然としてスコープ外であり、配列の先頭要素だけを読む誤ったコードを
/// 黙って生成してはならないこと。
#[test]
fn bare_array_reference_as_a_value_is_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR arr: ARRAY [1..3] OF INTEGER;
        VAR x: INTEGER;
        BEGIN
            WriteLn(arr)
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("a bare array reference must be rejected");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("array")),
        "diagnostics: {diagnostics:?}"
    );
}

/// 同じ配列型同士の丸ごと代入（`a := b`）も依然としてスコープ外であり、
/// 先頭要素だけを書き換える誤ったコードを黙って生成してはならないこと
/// （`wasd-sema`の`assignment_compatible`は構造的に同じ配列型同士の代入を
/// 許してしまうため、この段階（コード生成）でのガードが必要。
/// `CodeGenerator::gen_assignment`の`Some(ValueKind::Array(_))`腕の
/// ドキュメント参照）。
#[test]
fn whole_array_assignment_is_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR a: ARRAY [1..3] OF INTEGER;
        VAR b: ARRAY [1..3] OF INTEGER;
        BEGIN
            a := b
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("whole-array assignment must be rejected");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("whole-array")),
        "diagnostics: {diagnostics:?}"
    );
}

/// `PROCEDURE`/`FUNCTION`本体内のローカル配列変数は、`STRING[n]`と同様に
/// 依然としてスコープ外であること（`PROGRAM`直下のグローバル変数のみ
/// サポート）。
#[test]
fn local_array_variable_is_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        PROCEDURE Foo;
        VAR arr: ARRAY [1..3] OF INTEGER;
        BEGIN
        END;
        BEGIN
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("local array VARs must be rejected, not codegen'd");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("ARRAY")),
        "diagnostics: {diagnostics:?}"
    );
}

/// 多次元配列（`ARRAY [1..3] OF ARRAY [1..3] OF INTEGER`。`wasd-ast`は
/// これを入れ子の`Array`として表現する）は、要素型が`INTEGER`/`BOOLEAN`
/// のいずれでもないため、今回のスコープ外として報告されること。
#[test]
fn multi_dimensional_array_variable_is_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR grid: ARRAY [1..3] OF ARRAY [1..3] OF INTEGER;
        BEGIN
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("multi-dimensional arrays must be rejected");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("multi-dimensional")),
        "diagnostics: {diagnostics:?}"
    );
}

/// `INTEGER`/`BOOLEAN`以外を要素とする配列（例: `CHAR`）も今回のスコープ外
/// として報告されること。
#[test]
fn array_of_unsupported_element_type_is_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR letters: ARRAY [1..3] OF CHAR;
        BEGIN
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("arrays of unsupported element types must be rejected");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("array element type")),
        "diagnostics: {diagnostics:?}"
    );
}

/// 要素数が16bitワード数に収まらない配列は、パニックせず明確な診断で
/// 報告されること。
#[test]
fn array_with_too_many_elements_is_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR huge: ARRAY [1..70000] OF INTEGER;
        BEGIN
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("oversized arrays must be rejected, not codegen'd");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("too many elements")),
        "diagnostics: {diagnostics:?}"
    );
}
