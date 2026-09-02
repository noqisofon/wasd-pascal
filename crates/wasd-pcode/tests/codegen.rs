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
use wasd_pcode::{Address, CodeAddress, CodeGenerator, Opcode, PCodeModule, UnconfirmedOp};

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
            op(UnconfirmedOp::Str(Address(0))),
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
            op(UnconfirmedOp::Lod(Address(0))), // x
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Grt),
            op(UnconfirmedOp::Fjp(CodeAddress(7))), // -> else branch (index 7)
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Str(Address(0))),
            op(UnconfirmedOp::Ujp(CodeAddress(9))), // -> past else branch (STP)
            op(UnconfirmedOp::Ldc(2)),
            op(UnconfirmedOp::Str(Address(0))),
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
            op(UnconfirmedOp::Lod(Address(0))),
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Grt),
            op(UnconfirmedOp::Fjp(CodeAddress(6))), // -> STP, right after THEN body
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Str(Address(0))),
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
            op(UnconfirmedOp::Lod(Address(0))), // loop start (index 0)
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Grt),
            op(UnconfirmedOp::Fjp(CodeAddress(9))), // -> STP, past the loop
            op(UnconfirmedOp::Lod(Address(0))),
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Sbi),
            op(UnconfirmedOp::Str(Address(0))),
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
            op(UnconfirmedOp::Str(Address(0))), // i := 1
            op(UnconfirmedOp::Ldc(10)),
            op(UnconfirmedOp::Str(Address(1))), // hidden limit := 10
            op(UnconfirmedOp::Lod(Address(0))), // loop start (index 4)
            op(UnconfirmedOp::Lod(Address(1))),
            op(UnconfirmedOp::Leq),                  // i <= limit
            op(UnconfirmedOp::Fjp(CodeAddress(15))), // -> STP, past the loop
            op(UnconfirmedOp::Lod(Address(0))),      // body: i := i
            op(UnconfirmedOp::Str(Address(0))),
            op(UnconfirmedOp::Lod(Address(0))),
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Adi), // i + 1
            op(UnconfirmedOp::Str(Address(0))),
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
            op(UnconfirmedOp::Lod(Address(0))), // outer IF condition (index 0)
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Grt),
            op(UnconfirmedOp::Fjp(CodeAddress(13))), // outer IF -> STP
            op(UnconfirmedOp::Lod(Address(0))),      // inner WHILE condition (loop start, index 4)
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Grt),
            op(UnconfirmedOp::Fjp(CodeAddress(13))), // inner WHILE -> STP
            op(UnconfirmedOp::Lod(Address(0))),
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Sbi),
            op(UnconfirmedOp::Str(Address(0))),
            op(UnconfirmedOp::Ujp(CodeAddress(4))), // inner WHILE -> back to loop start
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
            op(UnconfirmedOp::Lod(Address(0))), // loop start (index 0): body
            op(UnconfirmedOp::Ldc(1)),
            op(UnconfirmedOp::Sbi),
            op(UnconfirmedOp::Str(Address(0))),
            op(UnconfirmedOp::Lod(Address(0))), // UNTIL x = 0
            op(UnconfirmedOp::Ldc(0)),
            op(UnconfirmedOp::Equ),
            op(UnconfirmedOp::Fjp(CodeAddress(0))), // false -> back to loop start
            op(UnconfirmedOp::Stp),
        ]
    );
}

/// 6. 今回のスコープ外の構文（`PROCEDURE`宣言等）を含むASTを渡した場合、
///    パニックせず明確なエラーが返ること。
#[test]
fn procedure_declarations_are_reported_as_an_error_without_panicking() {
    let program = parse_program(
        r#"
        PROGRAM P;
        PROCEDURE Foo;
        BEGIN
        END;
        BEGIN
        END.
        "#,
    );

    let result = CodeGenerator::new().generate(&program);
    let diagnostics = result.expect_err("PROCEDURE declarations must be rejected, not codegen'd");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("PROCEDURE") || d.message.contains("FUNCTION")),
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
