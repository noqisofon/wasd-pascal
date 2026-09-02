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
    KERNEL_SEGMENT,
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

/// 配列・レコード・ポインタ型の`VAR`宣言もスコープ外としてエラー報告
/// されること。
#[test]
fn array_typed_variables_are_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        VAR arr: ARRAY [1..10] OF INTEGER;
        BEGIN
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("array VARs must be rejected, not codegen'd");
    assert!(
        diagnostics.iter().any(|d| d.message.contains("ARRAY")),
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
    let diagnostics =
        result.expect_err("multi-argument WriteLn must be rejected, not codegen'd");
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
    assert!(geq_not, "expected GEQI immediately followed by NOT for `<`, got {ops:?}");
    // `a > b` -> LEQI + NOT (in that order, adjacent).
    let leq_not = ops
        .windows(2)
        .any(|w| w == [op(UnconfirmedOp::Leq), op(UnconfirmedOp::Not)]);
    assert!(leq_not, "expected LEQI immediately followed by NOT for `>`, got {ops:?}");
}
