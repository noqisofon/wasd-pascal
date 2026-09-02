//! ASTからp-code命令列を生成するコード生成器。
//!
//! # 今回のスコープ
//!
//! `INTEGER`/`BOOLEAN`型の変数・定数、算術演算（`+ - * DIV MOD`）、比較演算、
//! 論理演算（`AND OR NOT`）、代入文、`IF`/`WHILE`/`REPEAT UNTIL`/`FOR`に
//! よる制御構造、`BEGIN...END`の複合文、および最小限の
//! `PROGRAM ... BEGIN ... END.`全体構造のみを扱う。
//!
//! `PROCEDURE`/`FUNCTION`、`CASE`、`UNIT`、配列・レコード・ポインタ型、
//! `REAL`/`CHAR`型、組み込み手続きは意味解析を通過済みのASTに含まれ得るが、
//! 本クレートの責務では**ない**。遭遇した場合はパニックせず、
//! 「未対応機能」の[`wasd_ast::Diagnostic`]を積んでコード生成のみ諦める
//! （呼び出し元はレキサ・パーサー・意味解析と同様、`Result::Err`として
//! 診断の集合を受け取る）。
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
    BinOp, Block, ConstDecl, Diagnostic, Expr, ForDirection, Identifier, Literal, Program,
    Severity, Span, Statement, TypeExpr, UnOp, VarDecl,
};

use crate::ir::{Instruction, PCodeModule};
use crate::opcode::{Address, CodeAddress, Opcode, UnconfirmedOp};

/// 未確定のジャンプ先を持つ命令のインデックス。[`CodeGenerator::patch_jump`]
/// に渡してバックパッチする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingJump(usize);

/// グローバル変数1件の情報。
#[derive(Debug, Clone, Copy)]
struct VarSlot {
    address: Address,
}

/// `CONST`宣言の値。今回のスコープでは`INTEGER`/`BOOLEAN`のみ。
#[derive(Debug, Clone, Copy)]
enum ConstValue {
    Int(i64),
    Bool(bool),
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
}

fn normalize(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl CodeGenerator {
    pub fn new() -> Self {
        Self::default()
    }

    /// ASTからp-codeを生成する。今回のスコープ外の構文（`PROCEDURE`等）に
    /// 遭遇した場合、パニックせずエラーとして報告する。
    pub fn generate(&mut self, program: &Program) -> Result<PCodeModule, Vec<Diagnostic>> {
        self.instructions.clear();
        self.diagnostics.clear();
        self.vars.clear();
        self.consts.clear();
        self.next_address = 0;

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
        if !program.proc_decls.is_empty() || !program.func_decls.is_empty() {
            self.error(
                program.span,
                "PROCEDURE/FUNCTION code generation is out of scope for this step's minimal \
                 codegen",
            );
        }

        self.declare_consts(&program.const_decls);
        self.declare_vars(&program.var_decls);

        self.gen_block(&program.body);
        self.emit(UnconfirmedOp::Stp.into(), program.span);

        if self.diagnostics.is_empty() {
            Ok(PCodeModule {
                instructions: std::mem::take(&mut self.instructions),
                global_data_words: self.next_address,
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
            let supported = matches!(decl.ty, TypeExpr::Integer(_) | TypeExpr::Boolean(_));
            if !supported {
                self.error(
                    decl.ty.span(),
                    format!(
                        "VAR type '{}' is out of scope for this step's minimal codegen (only \
                         INTEGER/BOOLEAN are supported)",
                        describe_type(&decl.ty)
                    ),
                );
                continue;
            }
            for name in &decl.names {
                let address = self.alloc_slot();
                self.vars.insert(normalize(&name.name), VarSlot { address });
            }
        }
    }

    fn alloc_slot(&mut self) -> Address {
        let address = Address(self.next_address);
        self.next_address += 1;
        address
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

    /// [`Self::emit_pending_jump`]で発行した分岐命令のジャンプ先を
    /// 確定させる（バックパッチ）。
    fn patch_jump(&mut self, jump: PendingJump, target: CodeAddress) {
        let opcode = &mut self.instructions[jump.0].opcode;
        *opcode
            .jump_target_mut()
            .expect("PendingJump must always point at a UJP/FJP instruction") = target;
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
            Statement::ProcCall { name, span, .. } => {
                self.error(
                    *span,
                    format!(
                        "procedure calls ('{}') are out of scope for this step's minimal \
                         codegen",
                        name.name
                    ),
                );
            }
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
        let Some(slot) = self.vars.get(&normalize(&ident.name)).copied() else {
            self.error(
                ident.span,
                format!("'{}' is not a known variable in this scope", ident.name),
            );
            return;
        };
        self.gen_expr(value);
        self.emit(UnconfirmedOp::Str(slot.address).into(), span);
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
        let Some(slot) = self.vars.get(&normalize(&var.name)).copied() else {
            self.error(
                var.span,
                format!("'{}' is not a known variable in this scope", var.name),
            );
            return;
        };

        self.gen_expr(start);
        self.emit(UnconfirmedOp::Str(slot.address).into(), span);

        // 終了値はISO Pascalの規定通りループ開始前に一度だけ評価し、
        // 隠し一時変数に保持する（ループ本体に副作用があっても、毎回
        // 再評価されて終了条件がずれることを防ぐ）。
        let limit = self.alloc_slot();
        self.gen_expr(end);
        self.emit(UnconfirmedOp::Str(limit).into(), span);

        let loop_start = self.here();
        self.emit(UnconfirmedOp::Lod(slot.address).into(), span);
        self.emit(UnconfirmedOp::Lod(limit).into(), span);
        let continue_test = match direction {
            ForDirection::To => UnconfirmedOp::Leq,
            ForDirection::DownTo => UnconfirmedOp::Geq,
        };
        self.emit(continue_test.into(), span);
        let exit_loop = self.emit_pending_jump(UnconfirmedOp::Fjp, span);

        self.gen_stmt(body);

        self.emit(UnconfirmedOp::Lod(slot.address).into(), span);
        self.emit(UnconfirmedOp::Ldc(1).into(), span);
        let step = match direction {
            ForDirection::To => UnconfirmedOp::Adi,
            ForDirection::DownTo => UnconfirmedOp::Sbi,
        };
        self.emit(step.into(), span);
        self.emit(UnconfirmedOp::Str(slot.address).into(), span);
        self.emit(UnconfirmedOp::Ujp(loop_start).into(), span);
        self.patch_jump(exit_loop, self.here());
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
            Expr::RealLiteral(_, span) => {
                self.unsupported_expr(*span, "REAL literals");
            }
            Expr::StringLiteral(_, span) => {
                self.unsupported_expr(*span, "STRING literals");
            }
            Expr::NilLiteral(span) => {
                self.unsupported_expr(*span, "NIL / pointer values");
            }
            Expr::FuncCall { span, .. } => {
                self.unsupported_expr(*span, "function calls");
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

    fn gen_identifier_load(&mut self, ident: &Identifier) {
        let key = normalize(&ident.name);
        if let Some(value) = self.consts.get(&key).copied() {
            match value {
                ConstValue::Int(v) => self.emit_ldc_int(v, ident.span),
                ConstValue::Bool(v) => {
                    self.emit(UnconfirmedOp::Ldc(if v { 1 } else { 0 }).into(), ident.span);
                }
            }
            return;
        }
        if let Some(slot) = self.vars.get(&key).copied() {
            self.emit(UnconfirmedOp::Lod(slot.address).into(), ident.span);
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

    fn emit_binop(&mut self, op: BinOp, span: Span) {
        let opcode = match op {
            BinOp::Add => UnconfirmedOp::Adi,
            BinOp::Sub => UnconfirmedOp::Sbi,
            BinOp::Mul => UnconfirmedOp::Mpi,
            BinOp::IntDiv => UnconfirmedOp::Dvi,
            BinOp::Mod => UnconfirmedOp::Mod,
            BinOp::Eq => UnconfirmedOp::Equ,
            BinOp::NotEq => UnconfirmedOp::Neq,
            BinOp::Lt => UnconfirmedOp::Les,
            BinOp::Gt => UnconfirmedOp::Grt,
            BinOp::LtEq => UnconfirmedOp::Leq,
            BinOp::GtEq => UnconfirmedOp::Geq,
            BinOp::And => UnconfirmedOp::And,
            BinOp::Or => UnconfirmedOp::Ior,
            BinOp::Div => {
                self.error(
                    span,
                    "real division ('/') is out of scope for this step's minimal codegen (only \
                     INTEGER/BOOLEAN are supported)",
                );
                UnconfirmedOp::Ldc(0)
            }
        };
        self.emit(opcode.into(), span);
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
