//! ASTからp-code命令列を生成するコード生成器。
//!
//! # 今回のスコープ
//!
//! `INTEGER`/`BOOLEAN`型の変数・定数、算術演算（`+ - * DIV MOD`）、比較演算、
//! 論理演算（`AND OR NOT`）、代入文、`IF`/`WHILE`/`REPEAT UNTIL`/`FOR`に
//! よる制御構造、`BEGIN...END`の複合文、`PROGRAM ... BEGIN ... END.`
//! 全体構造、`PROGRAM`直下に宣言された`PROCEDURE`/`FUNCTION`の呼び出し、
//! および組み込み手続き`WriteLn`（引数0個または1個。`INTEGER`/`BOOLEAN`
//! に加え、Step 15から文字列リテラルの直接渡し（`WriteLn('...')`）も
//! サポート。[`CodeGenerator::gen_writeln_call`]参照）を扱う。文字列リテラルは
//! `WriteLn`への直接渡し以外の文脈では引き続きスコープ外（意味解析
//! （`wasd-sema`）の段階で型エラーになるため、本クレートまで到達しない）。
//!
//! `CASE`、`UNIT`、配列・レコード・ポインタ型、`REAL`/`CHAR`型、`WriteLn`
//! 以外の組み込み手続き（`Write`/`Read`/`ReadLn`/`New`/`Dispose`）、複数
//! 引数の`WriteLn`は意味解析を通過済みのASTに含まれ得るが、本クレートの
//! 責務では**ない**。遭遇した場合はパニックせず、「未対応機能」の
//! [`wasd_ast::Diagnostic`]を積んでコード生成のみ諦める（呼び出し元は
//! レキサ・パーサー・意味解析と同様、`Result::Err`として診断の集合を
//! 受け取る）。
//!
//! # PROCEDURE/FUNCTIONのネストについて
//!
//! `wasd_ast::ProcDecl`/`wasd_ast::FuncDecl`はそれ自身の中に
//! `proc_decls`/`func_decls`を持たない（`PROGRAM`直下にのみ宣言できる）
//! ため、本クレートが扱うレキシカルネストは常にたかだか2段階
//! （`PROGRAM`本体 = lexレベル0、その直下の`PROCEDURE`/`FUNCTION`本体 =
//! lexレベル1）に限られる。呼び出し命令の選択（`CPL`/`CPG`/`CPI`。
//! [`crate::opcode::ConfirmedOp`]のドキュメント参照）や変数アドレッシング
//! （[`ResolvedVar`]）は、将来`PROCEDURE`内`PROCEDURE`のようなさらに
//! 深いネストがASTに追加されることを見越した一般的な形で書いてあるが、
//! 現状のASTの制約上、実際に使われるのは常にlexレベル1向けの経路
//! （呼び出しは`CPG`、レベル差は0または1）のみである。
//!
//! # 活性化レコードのレイアウト
//!
//! `PROCEDURE`/`FUNCTION`ごとの活性化レコードは、Internal Architecture
//! Guide（Section II.4.2.1.3、p.48-49）に記載の通り、低アドレスから
//! 高アドレスに向けて次の順で構成される（[`crate::opcode::ConfirmedOp`]
//! のドキュメント参照）:
//!
//! 1. マーク・スタック（5ワード、固定。本クレートはこの5ワード自体を
//!    直接アドレッシングすることはなく、単に先頭オフセット`5`として
//!    予約するのみ）
//! 2. ローカル変数・一時変数領域（`DATASIZE`ワード）: オフセット
//!    `5..5+DATASIZE`
//! 3. パラメータ領域: オフセット`5+DATASIZE..5+DATASIZE+P`
//!    （`P`は仮引数のワード数。`VAR`仮引数はアドレスを1ワードで格納し、
//!    それ以外の値仮引数（本クレートのスコープでは`INTEGER`/`BOOLEAN`の
//!    み）は値そのものを1ワードで格納する）
//! 4. 関数の戻り値領域（`FUNCTION`のみ、1ワード）: オフセット
//!    `5+DATASIZE+P`
//!
//! `DATASIZE`は宣言済みローカル変数の個数に加え、`FOR`文が導入する
//! 隠しループ終了値の一時変数の個数も含む。後者は本体を実際に生成する
//! 前に[`count_for_temps`]で数え上げ、パラメータのオフセットを本体生成前
//! （したがって隠し一時変数がいくつ必要になるか判明する前）に確定できる
//! ようにしている。
//!
//! # 呼び出し規約とRPUのBパラメータ
//!
//! 呼び出し側は、呼び出し前に仮引数の並び順で引数を評価してスタックへ
//! 積む（`VAR`仮引数はアドレス、それ以外は値。[`CodeGenerator::gen_call_args`]
//! 参照）。呼び出し先本体の末尾では`RPU <b>`（[`crate::opcode::ConfirmedOp::Rpu`]
//! 参照）を発行し、活性化レコードを片付ける。
//!
//! `b`の正確な計算式は一次資料から完全には読み取れなかった
//! （[`crate::opcode::ConfirmedOp::Rpu`]のドキュメント参照）。本実装は
//! タスク依頼で示された方針A「`b` = `DATASIZE` + パラメータ領域の
//! ワード数」を採用する（[`CodeGenerator::emit_rpu`]参照）。これにより、
//! 呼び出し時に積んだパラメータ領域と本体が使ったローカル変数領域が
//! ちょうど切り詰められ、関数の戻り値領域（存在する場合）だけが
//! 呼び出し元に残る、という設計を意図している。Step 13で`pmachine-core`
//! を実装し、この方針Aで実際に`PROCEDURE`/`FUNCTION`呼び出しを実行して
//! 呼び出し前後のスタックポインタが期待通りに戻ることを確認した
//! （`crates/pmachine-core/tests/rpu_b_verification.rs`参照）。方針Aは
//! **本プロジェクトの実行モデル内ではCONFIRMED**である
//! （[`crate::opcode::ConfirmedOp::Rpu`]のドキュメント、および
//! `pmachine-core`のクレートドキュメントも参照。実機バイナリでの検証で
//! はない点に注意）。
//!
//! # 制御構造とラベル解決
//!
//! `IF`/`WHILE`/`REPEAT`/`FOR`は、分岐命令（`UJP`/`FJP`）とラベル
//! （ジャンプ先アドレス）の組み合わせで実装する。ラベル解決は
//! 「命令生成時は仮アドレス（`CodeAddress(0)`）を置き、ジャンプ先が
//! 判明した時点でバックパッチする」という一般的な方式を採る。前方分岐
//! （`IF`の`THEN`終端、`WHILE`のループ脱出等）はジャンプ先が生成時点では
//! 未確定なので仮アドレスを置いて後から[`CodeGenerator::patch_jump`]で
//! 書き換える。後方分岐（`WHILE`/`REPEAT`のループ先頭への戻り）は
//! ジャンプ先が生成時点で既に確定しているため、仮アドレスもバック
//! パッチも不要で直接ジャンプ先を書ける。
//!
//! 手続き/関数呼び出し（`CPG`等）の呼び出し先アドレスも同じ仕組みで
//! バックパッチする。相互再帰（`PROCEDURE A`が本体でまだ生成していない
//! `PROCEDURE B`を呼ぶ等）に対応するため、呼び出し先がまだ生成されて
//! いない場合は[`CodeGenerator::pending_calls`]に記録しておき、その
//! 呼び出し先の本体を実際に生成し始める瞬間（[`CodeGenerator::begin_routine_body`]）
//! に一括でバックパッチする。
//!
//! 制御構造の生成は再帰呼び出しで行い、各呼び出しが自分の仮アドレス・
//! パッチだけを扱う（グローバルなラベル表を持たない）ため、ネストした
//! 制御構造でもラベル解決が混線しない。
//!
//! # 式の評価順序
//!
//! p-machineはスタックマシンであるという設計に忠実に、式木を後順
//! （postorder）で辿りながら命令を生成する（二項演算なら左辺→右辺→
//! 演算子の順）。

use std::collections::HashMap;

use wasd_ast::{
    BinOp, Block, ConstDecl, Diagnostic, Expr, ForDirection, FuncDecl, Identifier, Literal,
    ParamDecl, ProcDecl, Program, Severity, Span, Statement, TypeExpr, UnOp, VarDecl,
};

use crate::builtin::{
    BUILTIN_WRITELN_BOOL, BUILTIN_WRITELN_INT, BUILTIN_WRITELN_NONE, BUILTIN_WRITELN_STRING,
    KERNEL_SEGMENT,
};
use crate::ir::{Instruction, PCodeModule, RoutineMeta};
use crate::opcode::{Address, CodeAddress, ConfirmedOp, Level, Opcode, UnconfirmedOp};

/// 未確定のジャンプ先・呼び出し先を持つ命令のインデックス。
/// [`CodeGenerator::patch_jump`]に渡してバックパッチする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingJump(usize);

/// 式・変数・定数・`FUNCTION`戻り値の「種類」。本クレートのスコープは
/// `INTEGER`/`BOOLEAN`の2型のみなので、`wasd_sema::Type`のような一般的な
/// 型表現ではなく、この最小限の2値enumで済ませる。`WriteLn(expr)`が
/// `BUILTIN_WRITELN_INT`/`BUILTIN_WRITELN_BOOL`のどちらを呼ぶべきかを
/// 決めるためだけに使う（[`CodeGenerator::infer_expr_kind`]参照）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Int,
    Bool,
}

/// `TypeExpr`をこのクレートのスコープが対応する[`ValueKind`]へ変換する。
/// `INTEGER`/`BOOLEAN`以外（このクレートのスコープ外の型）は`None`。
fn value_kind_of(ty: &TypeExpr) -> Option<ValueKind> {
    match ty {
        TypeExpr::Integer(_) => Some(ValueKind::Int),
        TypeExpr::Boolean(_) => Some(ValueKind::Bool),
        _ => None,
    }
}

/// グローバル変数1件の情報。
#[derive(Debug, Clone, Copy)]
struct VarSlot {
    address: Address,
    kind: ValueKind,
}

/// `CONST`宣言の値。今回のスコープでは`INTEGER`/`BOOLEAN`のみ。
#[derive(Debug, Clone, Copy)]
enum ConstValue {
    Int(i64),
    Bool(bool),
}

impl ConstValue {
    fn kind(self) -> ValueKind {
        match self {
            ConstValue::Int(_) => ValueKind::Int,
            ConstValue::Bool(_) => ValueKind::Bool,
        }
    }
}

/// 活性化レコード内の1スロット（ローカル変数または仮引数）の情報。
/// `by_ref`が真の場合、そのスロットに格納されているのは値ではなく
/// 呼び出し元の変数のアドレスである（`VAR`仮引数）。
#[derive(Debug, Clone, Copy)]
struct FrameSlot {
    address: Address,
    by_ref: bool,
    kind: ValueKind,
}

/// 識別子1つの参照先が解決した結果。ロード/ストア/アドレス取得の
/// いずれの操作にも必要な情報（レベル差・アドレス・`VAR`仮引数かどうか）
/// をまとめて持つ。
#[derive(Debug, Clone, Copy)]
struct ResolvedVar {
    level: Level,
    address: Address,
    by_ref: bool,
}

/// `PROCEDURE`/`FUNCTION`1件のメタデータ。宣言（呼び出し元から見える形の
/// シグネチャ）と本体生成（呼び出し先自身が使う情報）の両方に使う。
#[derive(Debug, Clone)]
struct RoutineInfo {
    /// 本体の先頭命令のアドレス。まだ本体を生成していない場合は
    /// `entry_known`が偽で、値は意味を持たない。
    entry: CodeAddress,
    entry_known: bool,
    /// 仮引数（呼び出し元の視点でのシグネチャ: 個数と`by_ref`のみ。
    /// 本クレートは型検査を行わないため型情報は持たない）。
    params: Vec<FrameSlot>,
    is_func: bool,
    /// `FUNCTION`の場合のみ、戻り値スロットのアドレス。
    return_address: Option<Address>,
    /// `FUNCTION`の場合のみ意味を持つ、戻り値の種類（`WriteLn(Foo())`の
    /// ような呼び出しで`BUILTIN_WRITELN_INT`/`BUILTIN_WRITELN_BOOL`の
    /// どちらを使うか決めるために使う。[`CodeGenerator::infer_expr_kind`]
    /// 参照）。`PROCEDURE`（`is_func`が偽）の場合は値に意味がない
    /// （`ValueKind::Int`をダミーとして入れる）。
    return_kind: ValueKind,
    /// `RPU`に渡す`b`の計算に使う、ローカル変数・一時変数領域の
    /// ワード数（[`crate::codegen`]モジュールドキュメントの`emit_rpu`
    /// 参照）。
    data_size: u16,
    /// `data_size`のうち、宣言済みローカル変数（`VAR`セクション）が
    /// 占めるワード数。残り（`data_size - declared_local_words`）が
    /// `FOR`文の隠しループ終了値等の一時変数の分。本体生成開始時に
    /// [`LocalScope::next_temp_address`]の初期値（`5 + declared_local_words`）
    /// を求めるために持つ。
    declared_local_words: u16,
    /// このルーチン自身の本体を生成する際に[`LocalScope::locals`]として
    /// インストールするマップ（仮引数名・ローカル変数名 → スロット）。
    locals: HashMap<String, FrameSlot>,
}

/// `PROCEDURE`/`FUNCTION`本体を生成している間だけ有効なスコープ状態。
#[derive(Debug, Clone)]
struct LocalScope {
    /// `FUNCTION`本体かどうか（`PROCEDURE`なら偽）。
    is_function: bool,
    /// 現在生成中のルーチン自身の名前（小文字正規化済み）。
    /// `FunctionName := value`形式の戻り値代入を検出するために使う。
    function_name: String,
    /// `FUNCTION`の場合のみ、戻り値スロットのアドレス。
    return_address: Option<Address>,
    /// 仮引数・宣言済みローカル変数のマップ（名前 → スロット）。
    locals: HashMap<String, FrameSlot>,
    /// 次に確保する一時変数（`FOR`文の隠しループ終了値等）のアドレス。
    /// 宣言済みローカル変数の直後から始まる。
    next_temp_address: u16,
}

/// ASTからp-codeを生成するコード生成器。1回の[`CodeGenerator::generate`]
/// 呼び出しごとに内部状態をリセットするため、インスタンスは使い回せる。
#[derive(Debug, Default)]
pub struct CodeGenerator {
    instructions: Vec<Instruction>,
    diagnostics: Vec<Diagnostic>,
    vars: HashMap<String, VarSlot>,
    consts: HashMap<String, ConstValue>,
    next_address: u16,
    routines: HashMap<String, RoutineInfo>,
    /// まだ本体が生成されていないルーチンへの呼び出し。ルーチン名
    /// （小文字正規化済み）ごとに、バックパッチが必要な命令の一覧を持つ。
    pending_calls: HashMap<String, Vec<PendingJump>>,
    /// `PROCEDURE`/`FUNCTION`本体を生成中の場合のみ`Some`。
    current_scope: Option<LocalScope>,
    /// 文字列定数プール。`WriteLn('...')`が積む文字列リテラルをここへ
    /// 追加し、そのインデックスを命令列から参照する
    /// （[`Self::gen_writeln_call`]、[`crate::ir::PCodeModule::string_pool`]
    /// のドキュメント参照）。
    string_pool: Vec<String>,
}

fn normalize(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl CodeGenerator {
    pub fn new() -> Self {
        Self::default()
    }

    /// ASTからp-codeを生成する。今回のスコープ外の構文に遭遇した場合、
    /// パニックせずエラーとして報告する。
    pub fn generate(&mut self, program: &Program) -> Result<PCodeModule, Vec<Diagnostic>> {
        self.instructions.clear();
        self.diagnostics.clear();
        self.vars.clear();
        self.consts.clear();
        self.next_address = 0;
        self.routines.clear();
        self.pending_calls.clear();
        self.current_scope = None;
        self.string_pool.clear();

        if !program.uses.is_empty() {
            self.error(
                program.span,
                "cross-unit code generation ('USES') is out of scope for this step's minimal \
                 codegen",
            );
        }
        if !program.type_decls.is_empty() {
            self.error(
                program.span,
                "TYPE declarations are out of scope for this step's minimal codegen (only \
                 built-in INTEGER/BOOLEAN variables are supported)",
            );
        }

        self.declare_consts(&program.const_decls);
        self.declare_vars(&program.var_decls);

        self.register_procs(&program.proc_decls);
        self.register_funcs(&program.func_decls);

        for proc in &program.proc_decls {
            self.gen_routine_body(&proc.name, &proc.body);
        }
        for func in &program.func_decls {
            self.gen_routine_body(&func.name, &func.body);
        }

        let entry = self.here();
        self.gen_block(&program.body);
        self.emit(UnconfirmedOp::Stp.into(), program.span);

        if self.diagnostics.is_empty() {
            let mut routines: Vec<RoutineMeta> = self
                .routines
                .values()
                .map(|info| RoutineMeta {
                    entry: info.entry,
                    param_count: info.params.len() as u16,
                    data_size: info.data_size,
                    is_func: info.is_func,
                })
                .collect();
            routines.sort_by_key(|r| r.entry.0);

            Ok(PCodeModule {
                instructions: std::mem::take(&mut self.instructions),
                global_data_words: self.next_address,
                routines,
                entry,
                string_pool: std::mem::take(&mut self.string_pool),
            })
        } else {
            Err(std::mem::take(&mut self.diagnostics))
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::new(span, Severity::Error, message));
    }

    fn declare_consts(&mut self, const_decls: &[ConstDecl]) {
        for decl in const_decls {
            let value = match &decl.value {
                Literal::Int(v, _) => ConstValue::Int(*v),
                Literal::Bool(v, _) => ConstValue::Bool(*v),
                Literal::Real(_, span) | Literal::Str(_, span) => {
                    self.error(
                        *span,
                        format!(
                            "CONST '{}': only INTEGER/BOOLEAN constants are supported by this \
                             step's minimal codegen",
                            decl.name.name
                        ),
                    );
                    continue;
                }
            };
            self.consts.insert(normalize(&decl.name.name), value);
        }
    }

    fn declare_vars(&mut self, var_decls: &[VarDecl]) {
        for decl in var_decls {
            let Some(kind) = value_kind_of(&decl.ty) else {
                self.error(
                    decl.ty.span(),
                    format!(
                        "VAR type '{}' is out of scope for this step's minimal codegen (only \
                         INTEGER/BOOLEAN are supported)",
                        describe_type(&decl.ty)
                    ),
                );
                continue;
            };
            for name in &decl.names {
                let address = self.alloc_slot();
                self.vars
                    .insert(normalize(&name.name), VarSlot { address, kind });
            }
        }
    }

    fn alloc_slot(&mut self) -> Address {
        let address = Address(self.next_address);
        self.next_address += 1;
        address
    }

    // ---- PROCEDURE/FUNCTION宣言の登録 ----

    /// 仮引数の型を検査し、活性化レコード内のオフセットを割り当てる。
    /// `param_base`は仮引数領域の先頭オフセット（`5 + data_size`）。
    /// 型がサポート外の仮引数は診断のみ積んでスロットを割り当てない
    /// （[`Self::declare_vars`]と同じ方針。この宣言全体はどのみち
    /// `Err`を返すことになるため、以降のオフセットのズレは問題にならない）。
    fn build_params(&mut self, params: &[ParamDecl], param_base: u16) -> Vec<(String, FrameSlot)> {
        let mut result = Vec::new();
        for p in params {
            let Some(kind) = value_kind_of(&p.ty) else {
                self.error(
                    p.ty.span(),
                    format!(
                        "parameter type '{}' is out of scope for this step's minimal codegen \
                         (only INTEGER/BOOLEAN are supported)",
                        describe_type(&p.ty)
                    ),
                );
                continue;
            };
            let slot = FrameSlot {
                address: Address(param_base + result.len() as u16),
                by_ref: p.by_ref,
                kind,
            };
            result.push((normalize(&p.name.name), slot));
        }
        result
    }

    /// 宣言済みローカル変数を検査し、活性化レコード内のオフセット
    /// （`5`起点）を割り当てる。返り値は割り当てた変数の個数
    /// （宣言済みローカル変数のワード数。隠し一時変数は含まない）。
    fn build_locals(
        &mut self,
        var_decls: &[VarDecl],
        locals: &mut HashMap<String, FrameSlot>,
    ) -> u16 {
        let mut count: u16 = 0;
        for decl in var_decls {
            let Some(kind) = value_kind_of(&decl.ty) else {
                self.error(
                    decl.ty.span(),
                    format!(
                        "local VAR type '{}' is out of scope for this step's minimal codegen \
                         (only INTEGER/BOOLEAN are supported)",
                        describe_type(&decl.ty)
                    ),
                );
                continue;
            };
            for name in &decl.names {
                let slot = FrameSlot {
                    address: Address(5 + count),
                    by_ref: false,
                    kind,
                };
                locals.insert(normalize(&name.name), slot);
                count += 1;
            }
        }
        count
    }

    /// 名前を登録する。同名のグローバル`PROCEDURE`/`FUNCTION`が既に
    /// 登録されている場合は診断のみ積み、最初の宣言を残す（`wasd-sema`の
    /// 「既に宣言されている」診断と同じ方針）。
    fn register_routine(
        &mut self,
        name: &Identifier,
        params: &[ParamDecl],
        var_decls: &[VarDecl],
        body: &Block,
        is_func: bool,
        return_kind: ValueKind,
    ) {
        let key = normalize(&name.name);
        if self.routines.contains_key(&key) {
            self.error(name.span, format!("'{}' is already declared", name.name));
            return;
        }

        let mut locals = HashMap::new();
        let declared_local_words = self.build_locals(var_decls, &mut locals);
        let temp_words = count_for_temps(body);
        let data_size = declared_local_words + temp_words;

        let param_base = 5 + data_size;
        let param_pairs = self.build_params(params, param_base);
        let params: Vec<FrameSlot> = param_pairs.iter().map(|(_, slot)| *slot).collect();
        for (name, slot) in param_pairs {
            locals.insert(name, slot);
        }

        let return_address = if is_func {
            Some(Address(param_base + params.len() as u16))
        } else {
            None
        };

        self.routines.insert(
            key,
            RoutineInfo {
                entry: CodeAddress(0),
                entry_known: false,
                params,
                is_func,
                return_address,
                return_kind,
                data_size,
                declared_local_words,
                locals,
            },
        );
    }

    fn register_procs(&mut self, procs: &[ProcDecl]) {
        for p in procs {
            // `PROCEDURE`に戻り値はないため`return_kind`はダミー
            // （`RoutineInfo::return_kind`のドキュメント参照）。
            self.register_routine(
                &p.name,
                &p.params,
                &p.var_decls,
                &p.body,
                false,
                ValueKind::Int,
            );
        }
    }

    fn register_funcs(&mut self, funcs: &[FuncDecl]) {
        for f in funcs {
            let return_kind = value_kind_of(&f.return_type);
            if return_kind.is_none() {
                self.error(
                    f.return_type.span(),
                    format!(
                        "FUNCTION return type '{}' is out of scope for this step's minimal \
                         codegen (only INTEGER/BOOLEAN are supported)",
                        describe_type(&f.return_type)
                    ),
                );
            }
            self.register_routine(
                &f.name,
                &f.params,
                &f.var_decls,
                &f.body,
                true,
                return_kind.unwrap_or(ValueKind::Int),
            );
        }
    }

    /// `PROCEDURE`/`FUNCTION`本体を生成する。ルーチン名が
    /// [`Self::register_routine`]で登録できていない場合（名前衝突等）は
    /// 何もしない。
    fn gen_routine_body(&mut self, name: &Identifier, body: &Block) {
        let key = normalize(&name.name);
        let Some(info) = self.routines.get(&key).cloned() else {
            return;
        };

        self.begin_routine_body(&key);

        let previous_scope = self.current_scope.replace(LocalScope {
            is_function: info.is_func,
            function_name: key.clone(),
            return_address: info.return_address,
            locals: info.locals.clone(),
            next_temp_address: 5 + info.declared_local_words,
        });

        self.gen_block(body);
        self.emit_rpu(&info, body.span);

        self.current_scope = previous_scope;
    }

    /// 呼び出し先の本体を生成し始める瞬間に呼ぶ。エントリアドレスを
    /// 確定させ、それまでに積み残していた呼び出し（[`Self::pending_calls`]）
    /// を一括でバックパッチする。
    fn begin_routine_body(&mut self, key: &str) {
        let entry = self.here();
        if let Some(info) = self.routines.get_mut(key) {
            info.entry = entry;
            info.entry_known = true;
        }
        if let Some(pending) = self.pending_calls.remove(key) {
            for jump in pending {
                self.patch_jump(jump, entry);
            }
        }
    }

    /// [`crate::opcode::ConfirmedOp::Rpu`]を発行する。
    ///
    /// # `b`の計算式（方針A、Step 13で実行検証済み）
    ///
    /// タスク依頼で示された2方針のうち方針A、「`b` = ローカル変数・
    /// 一時変数領域のワード数(`DATASIZE`) + パラメータ領域のワード数」を
    /// 採用する。Step 13で`pmachine-core`を実装し、この方針で呼び出し
    /// 前後のスタックポインタが期待通りに戻ることを実行検証した
    /// （[`crate::opcode::ConfirmedOp::Rpu`]のドキュメント参照。本
    /// プロジェクトの実行モデル内ではCONFIRMED、実機バイナリでの検証では
    /// ない点に注意）。
    fn emit_rpu(&mut self, info: &RoutineInfo, span: Span) {
        let b = info.data_size + info.params.len() as u16;
        self.emit(ConfirmedOp::Rpu(b).into(), span);
    }

    // ---- 変数解決 ----

    /// 識別子名（正規化済み）を、現在のスコープに応じてレベル差・
    /// アドレス・`VAR`仮引数かどうかへ解決する。ローカルスコープ
    /// （仮引数・ローカル変数）を最初に調べ、シャドーイングを反映する
    /// （見つからなければグローバル変数を調べる）。定数は対象外
    /// （[`Self::consts`]は別途扱う）。
    fn resolve_var(&self, key: &str) -> Option<ResolvedVar> {
        if let Some(scope) = &self.current_scope {
            if let Some(slot) = scope.locals.get(key) {
                return Some(ResolvedVar {
                    level: Level(0),
                    address: slot.address,
                    by_ref: slot.by_ref,
                });
            }
        }
        if let Some(slot) = self.vars.get(key) {
            let level = if self.current_scope.is_some() {
                Level(1)
            } else {
                Level(0)
            };
            return Some(ResolvedVar {
                level,
                address: slot.address,
                by_ref: false,
            });
        }
        None
    }

    /// 一時変数（`FOR`文の隠しループ終了値等）を1ワード確保する。
    /// 現在プロシージャ/関数本体を生成中であれば、そのローカル変数
    /// 領域から（[`LocalScope::next_temp_address`]）、そうでなければ
    /// グローバルデータ領域から確保する。
    fn alloc_temp(&mut self) -> ResolvedVar {
        if let Some(scope) = &mut self.current_scope {
            let address = Address(scope.next_temp_address);
            scope.next_temp_address += 1;
            ResolvedVar {
                level: Level(0),
                address,
                by_ref: false,
            }
        } else {
            let address = self.alloc_slot();
            ResolvedVar {
                level: Level(0),
                address,
                by_ref: false,
            }
        }
    }

    /// 解決済みの変数を読み込み、スタックへ値を積む。`VAR`仮引数
    /// （`by_ref`）の場合は、スロットに格納されているアドレスをまず
    /// [`UnconfirmedOp::Lod`]で読み、続けて[`UnconfirmedOp::Ind`]で
    /// 参照先の値をデリファレンスする。
    fn gen_load_resolved(&mut self, resolved: ResolvedVar, span: Span) {
        self.emit(
            UnconfirmedOp::Lod(resolved.level, resolved.address).into(),
            span,
        );
        if resolved.by_ref {
            self.emit(UnconfirmedOp::Ind.into(), span);
        }
    }

    /// 解決済みの変数へ値を格納する。`gen_value`が値を生成するコードを
    /// 発行するコールバックで、`by_ref`の場合は先にスロットからアドレスを
    /// 読み出してから値を積み、[`UnconfirmedOp::Sti`]で間接ストアする
    /// （アドレスが先、値が後というスタック順を仮定している。
    /// [`UnconfirmedOp::Sti`]のドキュメント参照）。
    fn gen_store_resolved(
        &mut self,
        resolved: ResolvedVar,
        span: Span,
        gen_value: impl FnOnce(&mut Self),
    ) {
        if resolved.by_ref {
            self.emit(
                UnconfirmedOp::Lod(resolved.level, resolved.address).into(),
                span,
            );
            gen_value(self);
            self.emit(UnconfirmedOp::Sti.into(), span);
        } else {
            gen_value(self);
            self.emit(
                UnconfirmedOp::Str(resolved.level, resolved.address).into(),
                span,
            );
        }
    }

    /// `VAR`引数として渡すため、解決済みの変数の**アドレス**をスタックへ
    /// 積む。対象自身が`VAR`仮引数（既に参照）である場合は、そのスロットに
    /// 格納されているアドレスをそのまま読み出して転送する（二重の
    /// アドレス取得を避ける。伝統的なPascalの「`VAR`引数をさらに別の
    /// `VAR`引数として渡す」場合の意味論）。それ以外は
    /// [`UnconfirmedOp::Lda`]で新たにアドレスを計算する。
    fn gen_address_of_resolved(&mut self, resolved: ResolvedVar, span: Span) {
        if resolved.by_ref {
            self.emit(
                UnconfirmedOp::Lod(resolved.level, resolved.address).into(),
                span,
            );
        } else {
            self.emit(
                UnconfirmedOp::Lda(resolved.level, resolved.address).into(),
                span,
            );
        }
    }

    fn emit(&mut self, opcode: Opcode, span: Span) -> usize {
        self.instructions.push(Instruction { opcode, span });
        self.instructions.len() - 1
    }

    fn here(&self) -> CodeAddress {
        CodeAddress(self.instructions.len() as u32)
    }

    /// 分岐命令（`UJP`/`FJP`）を仮アドレス（`CodeAddress(0)`）で発行し、
    /// 後で[`Self::patch_jump`]に渡すためのインデックスを返す。
    fn emit_pending_jump(
        &mut self,
        make_opcode: impl FnOnce(CodeAddress) -> UnconfirmedOp,
        span: Span,
    ) -> PendingJump {
        let idx = self.emit(make_opcode(CodeAddress(0)).into(), span);
        PendingJump(idx)
    }

    /// [`Self::emit_pending_jump`]で発行した分岐命令、または
    /// [`Self::emit_call`]で発行した呼び出し命令のジャンプ先/呼び出し先を
    /// 確定させる（バックパッチ）。
    fn patch_jump(&mut self, jump: PendingJump, target: CodeAddress) {
        let opcode = &mut self.instructions[jump.0].opcode;
        *opcode
            .jump_target_mut()
            .expect("PendingJump must always point at a backpatchable instruction") = target;
    }

    /// 呼び出し命令を発行する。呼び出し先の本体が既に生成済み
    /// （[`RoutineInfo::entry_known`]）であれば確定したアドレスへ、
    /// まだであれば仮アドレスで発行して[`Self::pending_calls`]に記録し、
    /// 後で[`Self::begin_routine_body`]がバックパッチする。
    ///
    /// 発行する命令は常に[`ConfirmedOp::Cpg`]である
    /// （[`crate::opcode::ConfirmedOp::Cpl`]のドキュメント参照:
    /// 本ステップのASTでは`PROCEDURE`/`FUNCTION`は常にlexレベル1にしか
    /// 宣言できないため）。
    fn emit_call(&mut self, callee_key: &str, span: Span) {
        let (entry_known, entry) = {
            let info = self
                .routines
                .get(callee_key)
                .expect("callee must already be registered by the time a call is emitted");
            (info.entry_known, info.entry)
        };
        if entry_known {
            self.emit(ConfirmedOp::Cpg(entry).into(), span);
        } else {
            let idx = self.emit(ConfirmedOp::Cpg(CodeAddress(0)).into(), span);
            self.pending_calls
                .entry(callee_key.to_string())
                .or_default()
                .push(PendingJump(idx));
        }
    }

    fn gen_block(&mut self, block: &Block) {
        for stmt in &block.statements {
            self.gen_stmt(stmt);
        }
    }

    fn gen_stmt(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Assignment {
                target,
                value,
                span,
            } => self.gen_assignment(target, value, *span),
            Statement::If {
                cond,
                then_branch,
                else_branch,
                span,
            } => self.gen_if(cond, then_branch, else_branch.as_deref(), *span),
            Statement::While { cond, body, span } => self.gen_while(cond, body, *span),
            Statement::For {
                var,
                start,
                end,
                direction,
                body,
                span,
            } => self.gen_for(var, start, end, *direction, body, *span),
            Statement::Repeat {
                body,
                until_cond,
                span,
            } => self.gen_repeat(body, until_cond, *span),
            Statement::Compound(block) => self.gen_block(block),
            Statement::CompilerDirective { .. } => {
                // コンパイラディレクティブは今回のスコープでは実行時の
                // 意味を持たない（`wasd-sema`のドキュメント参照）ため、
                // コード生成は何も行わない。
            }
            Statement::Case { span, .. } => {
                self.error(
                    *span,
                    "CASE statements are out of scope for this step's minimal codegen",
                );
            }
            Statement::ProcCall { name, args, span } => self.gen_proc_call(name, args, *span),
            // `Statement`は`#[non_exhaustive]`（`wasd-ast`のドキュメント
            // 参照）なので、他クレートである本クレートからのmatchには
            // ワイルドカード腕が必須。将来追加される未知の文バリアントは
            // パニックせずエラー報告のみ行う。
            _ => {
                self.error(
                    stmt.span(),
                    "this statement form is not supported by this step's minimal codegen",
                );
            }
        }
    }

    fn gen_assignment(&mut self, target: &Expr, value: &Expr, span: Span) {
        let Expr::Identifier(ident) = target else {
            self.error(
                target.span(),
                "assignment targets other than a simple variable (array/record/pointer \
                 lvalues) are out of scope for this step's minimal codegen",
            );
            return;
        };
        let key = normalize(&ident.name);

        // 伝統的なPascalの意味論: `FUNCTION`本体内での自分自身の名前への
        // 代入は、戻り値の設定を意味する（`wasd-sema`の
        // `check_assignment_to_identifier`と同じ解釈。専用の
        // `ReturnStatement`ノードは設けず、パーサーは通常の代入文として
        // パースし、意味解析・コード生成の双方でこの特別扱いを行う）。
        if let Some(scope) = &self.current_scope {
            if scope.is_function && scope.function_name == key {
                let return_address = scope
                    .return_address
                    .expect("function scope always has a return_address");
                self.gen_expr(value);
                self.emit(UnconfirmedOp::Str(Level(0), return_address).into(), span);
                return;
            }
        }

        match self.resolve_var(&key) {
            Some(resolved) => {
                self.gen_store_resolved(resolved, span, |g| g.gen_expr(value));
            }
            None => {
                if let Some(info) = self.routines.get(&key) {
                    if info.is_func {
                        self.error(
                            ident.span,
                            format!(
                                "Cannot assign to function '{}' outside of its own body",
                                ident.name
                            ),
                        );
                    } else {
                        self.error(
                            ident.span,
                            format!("Cannot assign to procedure '{}'", ident.name),
                        );
                    }
                } else {
                    self.error(
                        ident.span,
                        format!("'{}' is not a known variable in this scope", ident.name),
                    );
                }
                self.gen_expr(value);
            }
        }
    }

    fn gen_if(
        &mut self,
        cond: &Expr,
        then_branch: &Statement,
        else_branch: Option<&Statement>,
        span: Span,
    ) {
        self.gen_expr(cond);
        let skip_then = self.emit_pending_jump(UnconfirmedOp::Fjp, span);
        self.gen_stmt(then_branch);
        match else_branch {
            Some(else_branch) => {
                let skip_else = self.emit_pending_jump(UnconfirmedOp::Ujp, span);
                self.patch_jump(skip_then, self.here());
                self.gen_stmt(else_branch);
                self.patch_jump(skip_else, self.here());
            }
            None => {
                self.patch_jump(skip_then, self.here());
            }
        }
    }

    fn gen_while(&mut self, cond: &Expr, body: &Statement, span: Span) {
        let loop_start = self.here();
        self.gen_expr(cond);
        let exit_loop = self.emit_pending_jump(UnconfirmedOp::Fjp, span);
        self.gen_stmt(body);
        self.emit(UnconfirmedOp::Ujp(loop_start).into(), span);
        self.patch_jump(exit_loop, self.here());
    }

    fn gen_repeat(&mut self, body: &[Statement], until_cond: &Expr, span: Span) {
        let loop_start = self.here();
        for stmt in body {
            self.gen_stmt(stmt);
        }
        self.gen_expr(until_cond);
        // `UNTIL`条件が真になるまで繰り返す: 条件が偽の間はループ先頭へ
        // 戻る。戻り先はここで既に確定しているため、`FJP`は仮アドレスを
        // 経由せず直接発行できる（バックパッチ不要）。
        self.emit(UnconfirmedOp::Fjp(loop_start).into(), span);
    }

    fn gen_for(
        &mut self,
        var: &Identifier,
        start: &Expr,
        end: &Expr,
        direction: ForDirection,
        body: &Statement,
        span: Span,
    ) {
        let key = normalize(&var.name);
        let Some(resolved) = self.resolve_var(&key) else {
            self.error(
                var.span,
                format!("'{}' is not a known variable in this scope", var.name),
            );
            return;
        };

        self.gen_store_resolved(resolved, span, |g| g.gen_expr(start));

        // 終了値はISO Pascalの規定通りループ開始前に一度だけ評価し、
        // 隠し一時変数に保持する（ループ本体に副作用があっても、毎回
        // 再評価されて終了条件がずれることを防ぐ）。
        let limit = self.alloc_temp();
        self.gen_store_resolved(limit, span, |g| g.gen_expr(end));

        let loop_start = self.here();
        self.gen_load_resolved(resolved, span);
        self.gen_load_resolved(limit, span);
        let continue_test = match direction {
            ForDirection::To => UnconfirmedOp::Leq,
            ForDirection::DownTo => UnconfirmedOp::Geq,
        };
        self.emit(continue_test.into(), span);
        let exit_loop = self.emit_pending_jump(UnconfirmedOp::Fjp, span);

        self.gen_stmt(body);

        self.gen_store_resolved(resolved, span, |g| {
            g.gen_load_resolved(resolved, span);
            g.emit(UnconfirmedOp::Ldc(1).into(), span);
            let step = match direction {
                ForDirection::To => UnconfirmedOp::Adi,
                ForDirection::DownTo => UnconfirmedOp::Sbi,
            };
            g.emit(step.into(), span);
        });
        self.emit(UnconfirmedOp::Ujp(loop_start).into(), span);
        self.patch_jump(exit_loop, self.here());
    }

    /// 手続き呼び出し文。組み込み手続きのうち`WriteLn`のみ、
    /// [`Self::gen_writeln_call`]（[`crate::opcode::ConfirmedOp::Cxg`]
    /// 経由の簡略化されたKERNEL呼び出し）として実際に動作する。それ以外の
    /// 組み込み手続き（`Write`/`Read`/`ReadLn`/`New`/`Dispose`）は引き続き
    /// このクレートのスコープ外としてエラー報告する。それ以外の名前は、
    /// ユーザー定義の`PROCEDURE`として解決を試みる。
    fn gen_proc_call(&mut self, name: &Identifier, args: &[Expr], span: Span) {
        let key = normalize(&name.name);
        if key == "writeln" {
            self.gen_writeln_call(args, span);
            return;
        }

        const BUILTINS: &[&str] = &["write", "read", "readln", "new", "dispose"];
        if BUILTINS.contains(&key.as_str()) {
            self.error(
                span,
                format!(
                    "built-in procedure '{}' is out of scope for this step's minimal codegen",
                    name.name
                ),
            );
            for arg in args {
                self.gen_expr(arg);
            }
            return;
        }

        match self.routines.get(&key).cloned() {
            Some(info) if !info.is_func => {
                self.gen_call_args(&info.params, args, name, span);
                self.emit_call(&key, span);
            }
            Some(_) => {
                self.error(
                    name.span,
                    format!(
                        "'{}' is a function; a function cannot be called as a statement (its \
                         return value would be discarded)",
                        name.name
                    ),
                );
                for arg in args {
                    self.gen_expr(arg);
                }
            }
            None => {
                self.error(name.span, format!("Undefined procedure '{}'", name.name));
                for arg in args {
                    self.gen_expr(arg);
                }
            }
        }
    }

    /// `WriteLn`呼び出し文のコード生成。
    ///
    /// # 簡略化: 正式なUNITWRITE呼び出し規約の再現ではない
    ///
    /// [`crate::opcode::ConfirmedOp::Cxg`]・[`crate::builtin`]モジュール
    /// ドキュメント参照。ここでは「KERNELセグメントへの`CXG`呼び出し」と
    /// いう形だけを一次資料の階層構造から借り、実際のパラメータ渡しは
    /// 「出力する値（あれば）1ワードをスタックへ積んでおくだけ」という、
    /// 本クレート独自の単純化された規約を使う。
    ///
    /// - 引数なし（`WriteLn`）: 値を積まずに`BUILTIN_WRITELN_NONE`を呼ぶ
    ///   （改行のみ出力）。
    /// - 引数1つが文字列リテラル（`WriteLn('...')`）: `expr`自体を評価する
    ///   のではなく、文字列を[`Self::string_pool`]へ追加してそのインデックス
    ///   を（[`Self::emit_ldc_int`]で、簡略化した自前のプロトコルとして）
    ///   スタックへ積み、`BUILTIN_WRITELN_STRING`を呼ぶ（
    ///   [`crate::builtin::BUILTIN_WRITELN_STRING`]のドキュメント参照）。
    /// - 引数1つ（文字列リテラル以外の`WriteLn(expr)`）: `expr`を評価して
    ///   スタックへ積み、その式の種類（[`Self::infer_expr_kind`]）に応じて
    ///   `BUILTIN_WRITELN_INT`/`BUILTIN_WRITELN_BOOL`のいずれかを呼ぶ。
    ///   `INTEGER`/`BOOLEAN`以外（`REAL`等、今回のスコープ外）と推論された
    ///   場合はエラーを報告する。
    /// - 引数2つ以上: 今回のスコープ外としてエラーを報告する
    ///   （タスク依頼: 「複数引数`WriteLn(a, b, c)`は今回はサポートしなくて
    ///   よい」）。
    fn gen_writeln_call(&mut self, args: &[Expr], span: Span) {
        match args {
            [] => {
                self.emit(
                    ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_NONE).into(),
                    span,
                );
            }
            [Expr::StringLiteral(value, str_span)] => {
                let index = self.intern_string(value.clone());
                self.emit_ldc_int(index as i64, *str_span);
                self.emit(
                    ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_STRING).into(),
                    span,
                );
            }
            [arg] => {
                let kind = self.infer_expr_kind(arg);
                self.gen_expr(arg);
                let proc = match kind {
                    Some(ValueKind::Int) => BUILTIN_WRITELN_INT,
                    Some(ValueKind::Bool) => BUILTIN_WRITELN_BOOL,
                    None => {
                        self.error(
                            arg.span(),
                            "WriteLn only supports INTEGER/BOOLEAN/string literal arguments in \
                             this step's minimal codegen",
                        );
                        BUILTIN_WRITELN_INT
                    }
                };
                self.emit(ConfirmedOp::Cxg(KERNEL_SEGMENT, proc).into(), span);
            }
            _ => {
                self.error(
                    span,
                    "WriteLn with more than one argument is out of scope for this step's \
                     minimal codegen (only 0 or 1 argument(s) are supported)",
                );
                for arg in args {
                    self.gen_expr(arg);
                }
            }
        }
    }

    /// 文字列を[`Self::string_pool`]へ追加し、そのインデックスを返す
    /// （同じ文字列が複数回現れても重複排除はしない。単純な追記のみ）。
    fn intern_string(&mut self, value: String) -> usize {
        let index = self.string_pool.len();
        self.string_pool.push(value);
        index
    }

    /// 式の「種類」（[`ValueKind`]）を推論する。`WriteLn(expr)`が
    /// `BUILTIN_WRITELN_INT`/`BUILTIN_WRITELN_BOOL`のどちらを使うべきかを
    /// 決めるためだけに使う、本クレートのスコープ（`INTEGER`/`BOOLEAN`の
    /// 2型のみ）に限定した簡易な型推論。`wasd-sema`が既に行った型検査を
    /// 再現するものではない（本クレートは意味解析を経たASTを受け取る前提。
    /// [`crate`]モジュールドキュメント参照）。推論できない式（このクレートの
    /// スコープ外の式形、または`REAL`/`STRING`等）は`None`を返す。
    fn infer_expr_kind(&self, expr: &Expr) -> Option<ValueKind> {
        match expr {
            Expr::IntLiteral(..) | Expr::HexIntLiteral(..) => Some(ValueKind::Int),
            Expr::BoolLiteral(..) => Some(ValueKind::Bool),
            Expr::Identifier(ident) => self.lookup_kind(&normalize(&ident.name)),
            Expr::BinaryOp { op, .. } => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::IntDiv | BinOp::Mod => {
                    Some(ValueKind::Int)
                }
                BinOp::Eq
                | BinOp::NotEq
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::LtEq
                | BinOp::GtEq
                | BinOp::And
                | BinOp::Or => Some(ValueKind::Bool),
                BinOp::Div => None,
            },
            Expr::UnaryOp { op, .. } => match op {
                UnOp::Neg => Some(ValueKind::Int),
                UnOp::Not => Some(ValueKind::Bool),
            },
            Expr::Paren(inner, _) => self.infer_expr_kind(inner),
            Expr::FuncCall { name, .. } => {
                let info = self.routines.get(&normalize(&name.name))?;
                info.is_func.then_some(info.return_kind)
            }
            _ => None,
        }
    }

    /// 識別子1つ（正規化済みの名前）の種類を、[`Self::resolve_var`]と同じ
    /// 優先順位（ローカルスコープ→グローバル変数→定数→引数なし`FUNCTION`）
    /// で解決する。[`Self::infer_expr_kind`]の`Expr::Identifier`腕からのみ
    /// 使う。
    fn lookup_kind(&self, key: &str) -> Option<ValueKind> {
        if let Some(scope) = &self.current_scope {
            if let Some(slot) = scope.locals.get(key) {
                return Some(slot.kind);
            }
        }
        if let Some(slot) = self.vars.get(key) {
            return Some(slot.kind);
        }
        if let Some(value) = self.consts.get(key) {
            return Some(value.kind());
        }
        if let Some(info) = self.routines.get(key) {
            if info.is_func && info.params.is_empty() {
                return Some(info.return_kind);
            }
        }
        None
    }

    /// `FUNCTION`呼び出し式。呼び出し後、戻り値1ワードがスタックへ
    /// 残る（呼び出し先本体末尾の`RPU`が戻り値領域だけを残して活性化
    /// レコードを切り詰めるという設計。[`crate`]モジュールドキュメント
    /// 参照）。
    fn gen_func_call(&mut self, name: &Identifier, args: &[Expr], span: Span) {
        let key = normalize(&name.name);
        match self.routines.get(&key).cloned() {
            Some(info) if info.is_func => {
                self.gen_call_args(&info.params, args, name, span);
                self.emit_call(&key, span);
            }
            Some(_) => {
                self.error(
                    name.span,
                    format!(
                        "'{}' is a procedure; procedures cannot be used in an expression",
                        name.name
                    ),
                );
                for arg in args {
                    self.gen_expr(arg);
                }
                self.emit(UnconfirmedOp::Ldc(0).into(), span);
            }
            None => {
                self.error(name.span, format!("Undefined identifier '{}'", name.name));
                for arg in args {
                    self.gen_expr(arg);
                }
                self.emit(UnconfirmedOp::Ldc(0).into(), span);
            }
        }
    }

    /// 呼び出し引数を評価してスタックへ積む。`VAR`仮引数（`by_ref`）には
    /// アドレスを（[`Self::gen_address_of_resolved`]、単純な変数参照のみ
    /// サポート）、それ以外には値を（[`Self::gen_expr`]）積む。
    fn gen_call_args(
        &mut self,
        params: &[FrameSlot],
        args: &[Expr],
        callee: &Identifier,
        span: Span,
    ) {
        if args.len() != params.len() {
            self.error(
                span,
                format!(
                    "'{}' expects {} argument(s), found {}",
                    callee.name,
                    params.len(),
                    args.len()
                ),
            );
            for arg in args {
                self.gen_expr(arg);
            }
            return;
        }

        for (arg, param) in args.iter().zip(params.iter()) {
            if param.by_ref {
                self.gen_var_arg(arg, span);
            } else {
                self.gen_expr(arg);
            }
        }
    }

    fn gen_var_arg(&mut self, arg: &Expr, span: Span) {
        let Expr::Identifier(ident) = arg else {
            self.error(
                arg.span(),
                "cannot pass an expression as a VAR argument (only a simple variable reference \
                 is supported by this step's minimal codegen)",
            );
            self.gen_expr(arg);
            return;
        };
        let key = normalize(&ident.name);
        match self.resolve_var(&key) {
            Some(resolved) => self.gen_address_of_resolved(resolved, span),
            None => {
                self.error(
                    ident.span,
                    format!("'{}' is not a known variable in this scope", ident.name),
                );
                self.emit(UnconfirmedOp::Ldc(0).into(), span);
            }
        }
    }

    fn gen_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::IntLiteral(value, span) | Expr::HexIntLiteral(value, span) => {
                self.emit_ldc_int(*value, *span);
            }
            Expr::BoolLiteral(value, span) => {
                // UNCONFIRMED: TRUE=1/FALSE=0という表現の妥当性は
                // `crate::opcode::UnconfirmedOp`のドキュメント参照。
                self.emit(UnconfirmedOp::Ldc(if *value { 1 } else { 0 }).into(), *span);
            }
            Expr::Identifier(ident) => self.gen_identifier_load(ident),
            Expr::BinaryOp { op, lhs, rhs, span } => {
                self.gen_expr(lhs);
                self.gen_expr(rhs);
                self.emit_binop(*op, *span);
            }
            Expr::UnaryOp { op, operand, span } => {
                self.gen_expr(operand);
                let opcode = match op {
                    UnOp::Neg => UnconfirmedOp::Ngi,
                    UnOp::Not => UnconfirmedOp::Not,
                };
                self.emit(opcode.into(), *span);
            }
            Expr::Paren(inner, _) => self.gen_expr(inner),
            Expr::FuncCall { name, args, span } => self.gen_func_call(name, args, *span),
            Expr::RealLiteral(_, span) => {
                self.unsupported_expr(*span, "REAL literals");
            }
            Expr::StringLiteral(_, span) => {
                self.unsupported_expr(*span, "STRING literals");
            }
            Expr::NilLiteral(span) => {
                self.unsupported_expr(*span, "NIL / pointer values");
            }
            Expr::IndexAccess { span, .. } => {
                self.unsupported_expr(*span, "array indexing");
            }
            Expr::FieldAccess { span, .. } => {
                self.unsupported_expr(*span, "record field access");
            }
            Expr::Deref { span, .. } => {
                self.unsupported_expr(*span, "pointer dereference");
            }
            // `Expr`も`#[non_exhaustive]`。将来追加される未知の式バリアント
            // はパニックせずエラー報告のみ行う。
            _ => {
                self.unsupported_expr(expr.span(), "this expression form");
            }
        }
    }

    /// スコープ外の式を検出したときの共通処理。診断を1件積んだ上で、
    /// スタックの均衡を保つためのダミー値（`LDC 0`）を発行する
    /// （このモジュールから見て診断が1件でもあれば`generate`全体が
    /// `Err`を返すため、この命令列が実際に実行されることはない）。
    fn unsupported_expr(&mut self, span: Span, what: &str) {
        self.error(
            span,
            format!("{what} are out of scope for this step's minimal codegen"),
        );
        self.emit(UnconfirmedOp::Ldc(0).into(), span);
    }

    /// 識別子1つだけの式の読み込み。ローカル/グローバル変数、定数の順で
    /// 解決を試み、いずれでもなければ`PROCEDURE`/`FUNCTION`名として
    /// 解決を試みる。`FUNCTION`の名前が括弧なしで現れた場合は、伝統的な
    /// Pascalの慣習に従い引数なしの呼び出しとして扱う（`wasd-sema`の
    /// `infer_identifier_type`と同じ解釈）。
    fn gen_identifier_load(&mut self, ident: &Identifier) {
        let key = normalize(&ident.name);

        if let Some(resolved) = self.resolve_var(&key) {
            self.gen_load_resolved(resolved, ident.span);
            return;
        }
        if let Some(value) = self.consts.get(&key).copied() {
            match value {
                ConstValue::Int(v) => self.emit_ldc_int(v, ident.span),
                ConstValue::Bool(v) => {
                    self.emit(UnconfirmedOp::Ldc(if v { 1 } else { 0 }).into(), ident.span);
                }
            }
            return;
        }
        if let Some(info) = self.routines.get(&key).cloned() {
            if info.is_func {
                if info.params.is_empty() {
                    self.emit_call(&key, ident.span);
                } else {
                    self.error(
                        ident.span,
                        format!(
                            "Function '{}' expects {} argument(s), found 0",
                            ident.name,
                            info.params.len()
                        ),
                    );
                    self.emit(UnconfirmedOp::Ldc(0).into(), ident.span);
                }
            } else {
                self.error(
                    ident.span,
                    format!(
                        "'{}' is a procedure; procedures cannot be used in an expression",
                        ident.name
                    ),
                );
                self.emit(UnconfirmedOp::Ldc(0).into(), ident.span);
            }
            return;
        }
        self.error(
            ident.span,
            format!(
                "'{}' is not a known constant or variable in this scope",
                ident.name
            ),
        );
        self.emit(UnconfirmedOp::Ldc(0).into(), ident.span);
    }

    fn emit_ldc_int(&mut self, value: i64, span: Span) {
        match i16::try_from(value) {
            Ok(v) => {
                self.emit(UnconfirmedOp::Ldc(v).into(), span);
            }
            Err(_) => {
                self.error(
                    span,
                    format!(
                        "integer constant {value} does not fit in a 16-bit p-machine word \
                         (-32768..=32767)"
                    ),
                );
                self.emit(UnconfirmedOp::Ldc(0).into(), span);
            }
        }
    }

    /// # CONFIRMED: `<`/`>`は`GEQI`/`LEQI`+`NOT`で合成する
    ///
    /// 一次資料（[`crate::opcode::UnconfirmedOp::Equ`]のドキュメント参照。
    /// Section II.4.2.2.13）には`EQUI`/`NEQI`/`LEQI`/`GEQI`のみが確認でき、
    /// strictな`<`/`>`に対応するオペコードは存在しない。そのため:
    /// - `a < b` は `NOT (a >= b)` として、通常通り`lhs`→`rhs`の順で評価し
    ///   `GEQI`を発行した後に`NOT`を追加する（オペランドの並び順自体は
    ///   `>=`/`<=`と同じ。追加で必要になるのは否定のみ）。
    /// - `a > b` は `NOT (a <= b)` として、同様に`LEQI`+`NOT`で合成する。
    ///
    /// （なお「オペランドの順序を入れ替えるだけで`GEQI`から`<`が得られる」
    /// という単純化は数学的に誤り: `b >= a`は`a <= b`と同値であり、
    /// `a == b`の境界で`a < b`と食い違う。そのため本実装は常に`NOT`を
    /// 追加する、上記の正しい合成を採用する）。
    fn emit_binop(&mut self, op: BinOp, span: Span) {
        if matches!(op, BinOp::Div) {
            self.error(
                span,
                "real division ('/') is out of scope for this step's minimal codegen (only \
                 INTEGER/BOOLEAN are supported)",
            );
            self.emit(UnconfirmedOp::Ldc(0).into(), span);
            return;
        }

        let (opcode, negate) = match op {
            BinOp::Add => (UnconfirmedOp::Adi, false),
            BinOp::Sub => (UnconfirmedOp::Sbi, false),
            BinOp::Mul => (UnconfirmedOp::Mpi, false),
            BinOp::IntDiv => (UnconfirmedOp::Dvi, false),
            BinOp::Mod => (UnconfirmedOp::Mod, false),
            BinOp::Eq => (UnconfirmedOp::Equ, false),
            BinOp::NotEq => (UnconfirmedOp::Neq, false),
            BinOp::Lt => (UnconfirmedOp::Geq, true),
            BinOp::Gt => (UnconfirmedOp::Leq, true),
            BinOp::LtEq => (UnconfirmedOp::Leq, false),
            BinOp::GtEq => (UnconfirmedOp::Geq, false),
            BinOp::And => (UnconfirmedOp::And, false),
            BinOp::Or => (UnconfirmedOp::Ior, false),
            BinOp::Div => unreachable!("handled above"),
        };
        self.emit(opcode.into(), span);
        if negate {
            self.emit(UnconfirmedOp::Not.into(), span);
        }
    }
}

/// `body`の中に現れる`FOR`文の個数を再帰的に数える（隠しループ終了値の
/// 一時変数がいくつ必要になるかの見積もりに使う。[`crate`]モジュール
/// ドキュメントの「活性化レコードのレイアウト」参照）。`CASE`の各分岐は
/// スコープ外として実際には生成されない（[`CodeGenerator::gen_stmt`]
/// 参照）ため、意図的に数え上げの対象外とする。
fn count_for_temps(block: &Block) -> u16 {
    block.statements.iter().map(count_for_temps_stmt).sum()
}

fn count_for_temps_stmt(stmt: &Statement) -> u16 {
    match stmt {
        Statement::For { body, .. } => 1 + count_for_temps_stmt(body),
        Statement::If {
            then_branch,
            else_branch,
            ..
        } => {
            count_for_temps_stmt(then_branch)
                + else_branch
                    .as_deref()
                    .map(count_for_temps_stmt)
                    .unwrap_or(0)
        }
        Statement::While { body, .. } => count_for_temps_stmt(body),
        Statement::Repeat { body, .. } => body.iter().map(count_for_temps_stmt).sum(),
        Statement::Compound(block) => count_for_temps(block),
        _ => 0,
    }
}

fn describe_type(ty: &TypeExpr) -> &'static str {
    match ty {
        TypeExpr::Integer(_) => "INTEGER",
        TypeExpr::Real(_) => "REAL",
        TypeExpr::Boolean(_) => "BOOLEAN",
        TypeExpr::Char(_) => "CHAR",
        TypeExpr::StringN(..) => "STRING[n]",
        TypeExpr::Named(_) => "<named type>",
        TypeExpr::Array { .. } => "ARRAY",
        TypeExpr::Subrange { .. } => "<subrange>",
        TypeExpr::Record { .. } => "RECORD",
        TypeExpr::Pointer(..) => "<pointer>",
        _ => "<unknown type>",
    }
}
