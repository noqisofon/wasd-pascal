//! タスク4: `RPU`の`b`パラメータ（`crates/wasd-pcode/src/codegen.rs`の
//! `emit_rpu`が採用する方針A: `b = DATASIZE + パラメータ領域のワード数`）
//! の実行検証。
//!
//! 呼び出しの引数を積み始める前のSP（[`PMachine::sp`]）と、呼び出しから
//! 戻った直後（`RPU`実行後、呼び出し命令の次の命令に`IPC`が到達した時点）
//! のSPを比較する（`sp_around_first_call`参照。`CPG`命令自体の直前で
//! 測ると、既に引数の語数分スタックが伸びているため、その分を差し引く）。
//! `PROCEDURE`なら等しくなるはず、`FUNCTION`なら戻り値の1ワード分だけ
//! 大きくなるはず、というのが方針Aの意図（
//! `crates/pmachine-core/src/lib.rs`のモジュールドキュメント「タスク4」
//! 参照）。
//!
//! これらのテストが全て通ることをもって、方針Aは
//! **本クレートの実行モデル内ではCONFIRMED**として扱う。

mod common;

use pmachine_core::PMachine;
use wasd_pcode::{ConfirmedOp, Opcode};

/// ソース中で最初に現れる`CPG`命令の命令列インデックスと、その呼び出し先の
/// パラメータ語数（[`wasd_pcode::RoutineMeta::param_count`]）を返す。
fn first_call(module: &wasd_pcode::PCodeModule) -> (usize, u16) {
    let (index, target) = module
        .instructions
        .iter()
        .enumerate()
        .find_map(|(i, instr)| match instr.opcode {
            Opcode::Confirmed(ConfirmedOp::Cpg(target)) => Some((i, target)),
            _ => None,
        })
        .expect("source should contain at least one CPG instruction");
    let param_count = module
        .routines
        .iter()
        .find(|r| r.entry == target)
        .expect("call target must have routine metadata")
        .param_count;
    (index, param_count)
}

/// `module`を実行し、最初の呼び出しの**引数を積み始める前**のSPと、
/// 呼び出しから戻った直後のSPを`(before, after)`で返す。
///
/// 呼び出し元は、`CPG`本体を実行する前に仮引数の値（または`VAR`仮引数
/// ならアドレス）を評価スタックへ積む
/// （`crates/wasd-pcode/src/codegen.rs`の`gen_call_args`参照）ため、
/// `CPG`命令に到達した時点のSPは、既に引数の語数（`param_count`）分
/// だけ「呼び出し前」の高さより大きい。この関数は、そのぶんを差し引いた
/// 「引数評価を始める前のSP」を`before`として返すことで、
/// [`PMachine::sp`]の呼び出し前後の比較が意味を持つようにしている。
fn sp_around_first_call(module: wasd_pcode::PCodeModule) -> (usize, usize) {
    let (call_index, param_count) = first_call(&module);
    let mut vm = PMachine::new(module);

    while vm.ipc() != call_index {
        vm.step().expect("should not fail before reaching the call");
    }
    let sp_before = vm.sp() - param_count as usize;

    // CPG自体を実行する。IPCは呼び出し先のエントリへ飛ぶ。
    vm.step().expect("the call itself should not fail");

    // RPUが呼び出し元へ戻ると、IPCは`call_index + 1`（CPGの直後）に
    // なる。そこに到達するまでステップを進める。
    while vm.ipc() != call_index + 1 {
        vm.step()
            .expect("should not fail while executing the callee's body");
    }
    let sp_after = vm.sp();

    (sp_before, sp_after)
}

/// 引数なしの`PROCEDURE`呼び出し: 呼び出し前後でSPが完全に一致すること。
#[test]
fn no_arg_procedure_call_restores_sp_exactly() {
    let module = common::compile(
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

    let (before, after) = sp_around_first_call(module);
    assert_eq!(before, after, "a PROCEDURE call must leave SP unchanged");
}

/// 値引数を持つ`PROCEDURE`呼び出し: 呼び出し前に積んだ引数の分も含めて
/// SPが元に戻ること。
#[test]
fn value_parameter_procedure_call_restores_sp_exactly() {
    let module = common::compile(
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

    let (before, after) = sp_around_first_call(module);
    assert_eq!(before, after, "a PROCEDURE call must leave SP unchanged");
}

/// `VAR`引数を持つ`PROCEDURE`呼び出しでもSPが元に戻ること。
#[test]
fn var_parameter_procedure_call_restores_sp_exactly() {
    let module = common::compile(
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

    let (before, after) = sp_around_first_call(module);
    assert_eq!(before, after, "a PROCEDURE call must leave SP unchanged");
}

/// `FUNCTION`呼び出し: 戻り値1ワード分だけSPが呼び出し前より大きくなる
/// こと。
#[test]
fn function_call_leaves_exactly_one_return_value_word() {
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

    let (before, after) = sp_around_first_call(module);
    assert_eq!(
        after,
        before + 1,
        "a FUNCTION call must leave exactly the return value on the stack"
    );
}

/// 複数回の呼び出し（`PROCEDURE`を2回連続で呼ぶ）を経ても、SPが
/// 呼び出しごとに正しく元へ戻ること（1回だけたまたま合っている、という
/// 可能性を排除する）。
#[test]
fn repeated_procedure_calls_each_restore_sp() {
    let module = common::compile(
        r#"
        PROGRAM P;
        VAR x: INTEGER;
        PROCEDURE Inc(VAR n: INTEGER);
        BEGIN
            n := n + 1
        END;
        BEGIN
            x := 0;
            Inc(x);
            Inc(x);
            Inc(x)
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    let sp_at_start = vm.sp();
    vm.run().expect("program should run without error");
    // プログラム全体の実行が終わった時点で、SPはグローバルデータ領域
    // だけが残った状態（呼び出し・式評価の一時値はすべて消費済み）に
    // 戻っているはず。
    assert_eq!(vm.sp(), sp_at_start);
    assert_eq!(vm.global(0), Some(3));
}

/// 再帰呼び出しを経てもSPが正しく管理されること（各再帰段でスタックが
/// 破綻していれば、最終的な戻り値かSPのどちらかが必ずずれる）。
#[test]
fn recursive_calls_leave_sp_balanced_after_the_program_completes() {
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
            result := Fact(7)
        END.
        "#,
    );

    let mut vm = PMachine::new(module);
    let sp_at_start = vm.sp();
    vm.run().expect("program should run without error");
    assert_eq!(vm.sp(), sp_at_start, "no leftover frames or temporaries");
    assert_eq!(vm.call_depth(), 0, "all activation records must be popped");
    assert_eq!(vm.global(0), Some(5040));
}
