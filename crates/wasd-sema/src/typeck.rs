//! 型検査。
//!
//! `wasd-parser`が生成した`wasd_ast::Program`を走査し、型エラーを
//! `wasd_ast::Diagnostic`として報告する。dialectチェックはここでは行わない
//! （`wasd-sema`のクレートドキュメント参照）。
//!
//! # エラー耐性
//!
//! [`SemaContext::check_program`]はパニックせず、型エラーに遭遇しても
//! 走査を止めない。型エラーが発生した式・宣言には[`Type::Error`]を
//! 割り当て、それを使った以降の演算・比較・代入では追加の診断を
//! 出さないことで、1つのエラーが無関係な多数のエラーを誘発する
//! カスケードエラーを防ぐ。

use wasd_ast as ast;
use wasd_ast::{Diagnostic, Identifier, Severity, Span};

use crate::symbol_table::{ParamSignature, SymbolInfo, SymbolKind, SymbolTable};
use crate::types::Type;

/// 意味解析（今回のスコープでは型検査のみ）の実行コンテキスト。
pub struct SemaContext {
    symbol_table: SymbolTable,
    diagnostics: Vec<Diagnostic>,
    /// 現在型検査中の`FUNCTION`本体の名前（小文字正規化済み）。
    ///
    /// `FunctionName := value`という形の代入は「戻り値の設定」を意味するが、
    /// これが許されるのはその関数自身の本体の中だけである
    /// （他の関数やプログラム本体から他の関数の名前に代入することはできない）。
    /// この判定のために、現在どの`FUNCTION`の本体を解析中かを保持する。
    /// `PROCEDURE`本体やプログラム本体の解析中は`None`。
    current_function: Option<String>,
    /// 現在解析中の`FOR`文のループ制御変数名（小文字正規化済み）のスタック。
    /// ネストした`FOR`文それぞれのループ変数を積んでおく。
    ///
    /// # ループ変数をループ内で読み取り専用として扱う方針について
    ///
    /// ISO/IEC 7185:1990 6.8.3.9 "The for-statement"は、`for-statement`の
    /// 実行中に制御変数の値を（本体からの代入によって）変更した場合の動作を
    /// "erroneous"（不正）と定めている（規格が定める意味論上、変更後の
    /// 挙動は保証されない）。本実装ではこれを実行時エラーではなく
    /// コンパイル時エラーとして検出する方が診断として親切であるため、
    /// `FOR`文の本体内でループ変数への代入を検出した場合は
    /// `Severity::Error`の診断を出す。
    for_loop_vars: Vec<String>,
}

impl Default for SemaContext {
    fn default() -> Self {
        Self::new()
    }
}

impl SemaContext {
    pub fn new() -> Self {
        Self {
            symbol_table: SymbolTable::new(),
            diagnostics: Vec::new(),
            current_function: None,
            for_loop_vars: Vec::new(),
        }
    }

    /// `Program`を走査し、型検査を行う。
    ///
    /// `VAR`/`CONST`宣言をシンボルテーブルに登録したのち、`PROCEDURE`/
    /// `FUNCTION`宣言を順に処理し、最後にプログラム本体の`Block`を
    /// 型検査する。パニックせず、エラーがあっても可能な限り走査を継続して
    /// すべての`Diagnostic`を蓄積する。
    ///
    /// # `PROCEDURE`/`FUNCTION`の処理順序について
    ///
    /// `Program`は`proc_decls`/`func_decls`を別々の`Vec`として持つため、
    /// ソース上の宣言順序（`PROCEDURE`と`FUNCTION`が混在する順序）は
    /// 保持されない。ここでは`proc_decls`をすべて先に、続いて`func_decls`を
    /// すべて処理する。各宣言は「自分自身の名前をシンボルテーブルに登録して
    /// から本体を型検査する」ため、自分自身の再帰呼び出しは常に許可される。
    /// 一方、まだ処理していない（後から出てくる）`PROCEDURE`/`FUNCTION`を
    /// 呼び出すこと（相互再帰）は、`forward`宣言に相当する仕組みが
    /// 今回のスコープに無いため未対応であり、「Undefined
    /// procedure/function」として報告される。
    pub fn check_program(&mut self, program: &ast::Program) -> Vec<Diagnostic> {
        // 同じ`SemaContext`を使い回しても前回の解析結果を引きずらないよう、
        // 呼び出しのたびに状態をリセットする。
        self.symbol_table = SymbolTable::new();
        self.diagnostics = Vec::new();
        self.current_function = None;
        self.for_loop_vars = Vec::new();

        self.collect_const_decls(&program.const_decls);
        self.collect_var_decls(&program.var_decls);

        for proc in &program.proc_decls {
            self.check_proc_decl(proc);
        }
        for func in &program.func_decls {
            self.check_func_decl(func);
        }

        self.check_block(&program.body);

        std::mem::take(&mut self.diagnostics)
    }

    // ---- PROCEDURE/FUNCTION宣言 ----

    fn param_signatures(&self, params: &[ast::ParamDecl]) -> Vec<ParamSignature> {
        params
            .iter()
            .map(|p| ParamSignature {
                ty: type_from_type_expr(&p.ty),
                by_ref: p.by_ref,
            })
            .collect()
    }

    fn declare_params(&mut self, params: &[ast::ParamDecl]) {
        for p in params {
            let ty = type_from_type_expr(&p.ty);
            self.declare(&p.name, ty, SymbolKind::Param { by_ref: p.by_ref }, p.span);
        }
    }

    fn check_proc_decl(&mut self, decl: &ast::ProcDecl) {
        let params = self.param_signatures(&decl.params);
        // 自分自身の名前を、本体を型検査するより前に（本体用の内側スコープを
        // pushするより前に、つまり外側のスコープに）登録する。これにより
        // 本体内からの自分自身の再帰呼び出しが解決できるようになる。
        self.declare(
            &decl.name,
            Type::Error,
            SymbolKind::Proc { params },
            decl.span,
        );

        self.symbol_table.push_scope();
        self.declare_params(&decl.params);
        self.collect_var_decls(&decl.var_decls);

        let previous_function = self.current_function.take();
        self.check_block(&decl.body);
        self.current_function = previous_function;

        self.symbol_table.pop_scope();
    }

    fn check_func_decl(&mut self, decl: &ast::FuncDecl) {
        let params = self.param_signatures(&decl.params);
        let return_type = type_from_type_expr(&decl.return_type);
        self.declare(
            &decl.name,
            return_type,
            SymbolKind::Func {
                params,
                return_type,
            },
            decl.span,
        );

        self.symbol_table.push_scope();
        self.declare_params(&decl.params);
        self.collect_var_decls(&decl.var_decls);

        let previous_function = self
            .current_function
            .replace(decl.name.name.to_ascii_lowercase());
        self.check_block(&decl.body);
        self.current_function = previous_function;

        self.symbol_table.pop_scope();
    }

    // ---- 宣言の登録（1パス目） ----

    fn collect_const_decls(&mut self, decls: &[ast::ConstDecl]) {
        for decl in decls {
            let ty = self.infer_literal_type(&decl.value);
            self.declare(&decl.name, ty, SymbolKind::Const, decl.span);
        }
    }

    fn collect_var_decls(&mut self, decls: &[ast::VarDecl]) {
        for decl in decls {
            let ty = type_from_type_expr(&decl.ty);
            for name in &decl.names {
                self.declare(name, ty, SymbolKind::Var, decl.span);
            }
        }
    }

    fn declare(&mut self, name: &Identifier, ty: Type, kind: SymbolKind, declared_at: Span) {
        let info = SymbolInfo {
            ty,
            kind,
            declared_at,
        };
        if self.symbol_table.declare(&name.name, info).is_err() {
            self.diagnostics.push(Diagnostic::new(
                name.span,
                Severity::Error,
                format!("'{}' is already declared", name.name),
            ));
        }
    }

    // ---- 文の型検査（2パス目） ----

    fn check_block(&mut self, block: &ast::Block) {
        for stmt in &block.statements {
            self.check_statement(stmt);
        }
    }

    fn check_statement(&mut self, stmt: &ast::Statement) {
        match stmt {
            ast::Statement::Assignment { target, value, .. } => {
                self.check_assignment(target, value);
            }
            ast::Statement::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.check_condition(cond);
                self.check_statement(then_branch);
                if let Some(else_branch) = else_branch {
                    self.check_statement(else_branch);
                }
            }
            ast::Statement::While { cond, body, .. } => {
                self.check_condition(cond);
                self.check_statement(body);
            }
            ast::Statement::For {
                var,
                start,
                end,
                body,
                ..
            } => {
                self.check_for_statement(var, start, end, body);
            }
            ast::Statement::Repeat { body, until_cond, .. } => {
                for stmt in body {
                    self.check_statement(stmt);
                }
                // `WHILE`とは条件の意味が逆（「真になるまで」）だが、
                // 型検査の観点では「条件式はBOOLEANでなければならない」という
                // 制約自体は`WHILE`/`IF`と同じであるため`check_condition`を
                // 再利用する。真偽の評価方向（ループ継続条件か終了条件か）は
                // 実行時のコード生成の関心事であり、型検査には現れない。
                self.check_condition(until_cond);
            }
            ast::Statement::Case {
                selector,
                branches,
                ..
            } => {
                self.check_case_statement(selector, branches);
            }
            ast::Statement::Compound(block) => self.check_block(block),
            ast::Statement::ProcCall { name, args, .. } => {
                self.check_proc_call(name, args);
            }
            // `Statement`は`#[non_exhaustive]`。将来追加されるバリアントは、
            // 追加時にこの型検査を拡張するまでの間、ここでは何もしない
            // （このステップのスコープ外のため）。
            _ => {}
        }
    }

    /// `CASE selector OF label1, label2: stmt1; ... END`の型検査。
    ///
    /// - `selector`は順序型でなければならない。今回のスコープでは
    ///   `INTEGER`/`CHAR`/`BOOLEAN`のみをサポートする（`REAL`は不可）。
    /// - 各`label`の型は`selector`の型と一致しなければならない。
    /// - `label`はすべての分岐を通じて重複してはならない。
    ///
    /// # 方針: 非網羅性（どの分岐にも一致しない値が来た場合）は診断しない
    ///
    /// ISO/IEC 7185:1990はCASE文で選択子がどの`case-constant`にも一致しない
    /// 場合の動作を規定しておらず（実装依存/未定義）、コンパイル時に
    /// 「網羅性」を機械的に判定することも一般には困難である
    /// （`INTEGER`は事実上無限の値域を持つため、静的な網羅性チェックには
    /// 意味がない）。そのため本実装は非網羅性を診断しない
    /// （エラーにも警告にもしない）。UCSD拡張の`OTHERWISE`句
    /// （今回のスコープ外）を導入すれば、利用者が明示的にフォールバックを
    /// 書けるようになる。
    fn check_case_statement(&mut self, selector: &ast::Expr, branches: &[ast::CaseBranch]) {
        let selector_ty = self.infer_expr_type(selector);
        let selector_is_ordinal = matches!(selector_ty, Type::Integer | Type::Boolean | Type::Char);
        if !selector_is_ordinal && selector_ty != Type::Error {
            self.diagnostics.push(Diagnostic::new(
                selector.span(),
                Severity::Error,
                format!(
                    "CASE selector must be of an ordinal type (INTEGER/CHAR/BOOLEAN), found '{selector_ty}'"
                ),
            ));
        }

        let mut seen_labels: Vec<String> = Vec::new();

        for branch in branches {
            for label in &branch.labels {
                let label_ty = self.infer_literal_type(label);

                // `selector`自体が既に順序型エラーを起こしている場合、各ラベルに
                // 対して重ねて「型が一致しない」という追加の診断は出さない
                // （カスケードエラー防止）。
                if selector_is_ordinal && label_ty != Type::Error && label_ty != selector_ty {
                    self.diagnostics.push(Diagnostic::new(
                        label.span(),
                        Severity::Error,
                        format!(
                            "Type mismatch: CASE label of type '{label_ty}' does not match the selector type '{selector_ty}'"
                        ),
                    ));
                }

                let key = case_label_key(label);
                if seen_labels.contains(&key) {
                    self.diagnostics.push(Diagnostic::new(
                        label.span(),
                        Severity::Error,
                        format!("Duplicate CASE label {}", describe_case_label(label)),
                    ));
                } else {
                    seen_labels.push(key);
                }
            }
            self.check_statement(&branch.body);
        }
    }

    /// `FOR var := start (TO|DOWNTO) end DO body`の型検査。
    ///
    /// 今回のスコープではループ変数・`start`/`end`式ともに`INTEGER`型のみを
    /// サポートする（ISO 7185は任意の順序型を許すが、順序型全般
    /// （列挙型・部分範囲型など）は未実装のため対象外）。
    fn check_for_statement(
        &mut self,
        var: &Identifier,
        start: &ast::Expr,
        end: &ast::Expr,
        body: &ast::Statement,
    ) {
        match self.symbol_table.lookup(&var.name).cloned() {
            Some(info) => match info.kind {
                SymbolKind::Var | SymbolKind::Param { .. } => {
                    if info.ty != Type::Integer && info.ty != Type::Error {
                        self.diagnostics.push(Diagnostic::new(
                            var.span,
                            Severity::Error,
                            format!(
                                "FOR loop variable '{}' must be of type INTEGER, found '{}'",
                                var.name, info.ty
                            ),
                        ));
                    }
                }
                SymbolKind::Const | SymbolKind::Proc { .. } | SymbolKind::Func { .. } => {
                    self.diagnostics.push(Diagnostic::new(
                        var.span,
                        Severity::Error,
                        format!(
                            "'{}' is not a variable and cannot be used as a FOR loop control variable",
                            var.name
                        ),
                    ));
                }
            },
            None => {
                self.diagnostics.push(Diagnostic::new(
                    var.span,
                    Severity::Error,
                    format!("Undefined identifier '{}'", var.name),
                ));
            }
        }

        let start_ty = self.infer_expr_type(start);
        if start_ty != Type::Integer && start_ty != Type::Error {
            self.diagnostics.push(Diagnostic::new(
                start.span(),
                Severity::Error,
                format!("FOR loop start value must be of type INTEGER, found '{start_ty}'"),
            ));
        }

        let end_ty = self.infer_expr_type(end);
        if end_ty != Type::Integer && end_ty != Type::Error {
            self.diagnostics.push(Diagnostic::new(
                end.span(),
                Severity::Error,
                format!("FOR loop end value must be of type INTEGER, found '{end_ty}'"),
            ));
        }

        // ループ本体の型検査中、ループ変数への代入を禁止する
        // （`for_loop_vars`フィールドのドキュメント参照）。
        self.for_loop_vars.push(var.name.to_ascii_lowercase());
        self.check_statement(body);
        self.for_loop_vars.pop();
    }

    fn check_assignment(&mut self, target: &Identifier, value: &ast::Expr) {
        let value_ty = self.infer_expr_type(value);

        let lower_target = target.name.to_ascii_lowercase();
        if self.for_loop_vars.contains(&lower_target) {
            self.diagnostics.push(Diagnostic::new(
                target.span,
                Severity::Error,
                format!(
                    "Cannot assign to FOR loop control variable '{}' within the loop body \
                     (ISO/IEC 7185 6.8.3.9: altering the control variable during the \
                     execution of a for-statement is erroneous)",
                    target.name
                ),
            ));
            return;
        }

        match self.symbol_table.lookup(&target.name).cloned() {
            Some(info) => match info.kind {
                SymbolKind::Var | SymbolKind::Const | SymbolKind::Param { .. } => {
                    let target_ty = info.ty;
                    if !assignment_compatible(target_ty, value_ty) {
                        self.diagnostics.push(Diagnostic::new(
                            target.span,
                            Severity::Error,
                            format!(
                                "Type mismatch: cannot assign '{value_ty}' to variable '{}' of type '{target_ty}'",
                                target.name
                            ),
                        ));
                    }
                }
                // 伝統的なPascalの意味論: `FUNCTION`本体内での自分自身の名前への
                // 代入は、戻り値の設定を意味する（専用の`ReturnStatement`ノードは
                // 設けず、パーサーは通常の代入文としてパースし、ここ意味解析側で
                // 「関数名への代入」として解釈する）。この解釈が許されるのは
                // その関数自身の本体の中だけであり、他の関数やプログラム本体から
                // 任意の関数名に代入することはできない。
                SymbolKind::Func { return_type, .. } => {
                    let is_own_function = self
                        .current_function
                        .as_deref()
                        .is_some_and(|f| f == target.name.to_ascii_lowercase());
                    if is_own_function {
                        if !assignment_compatible(return_type, value_ty) {
                            self.diagnostics.push(Diagnostic::new(
                                target.span,
                                Severity::Error,
                                format!(
                                    "Type mismatch: cannot assign '{value_ty}' to the return value of function '{}' of type '{return_type}'",
                                    target.name
                                ),
                            ));
                        }
                    } else {
                        self.diagnostics.push(Diagnostic::new(
                            target.span,
                            Severity::Error,
                            format!(
                                "Cannot assign to function '{}' outside of its own body",
                                target.name
                            ),
                        ));
                    }
                }
                SymbolKind::Proc { .. } => {
                    self.diagnostics.push(Diagnostic::new(
                        target.span,
                        Severity::Error,
                        format!("Cannot assign to procedure '{}'", target.name),
                    ));
                }
            },
            None => {
                self.diagnostics.push(Diagnostic::new(
                    target.span,
                    Severity::Error,
                    format!("Undefined identifier '{}'", target.name),
                ));
            }
        }
    }

    fn check_condition(&mut self, cond: &ast::Expr) {
        let ty = self.infer_expr_type(cond);
        if ty != Type::Boolean && ty != Type::Error {
            self.diagnostics.push(Diagnostic::new(
                cond.span(),
                Severity::Error,
                format!("Condition must be of type BOOLEAN, found '{ty}'"),
            ));
        }
    }

    /// 手続き呼び出しの型検査。
    ///
    /// 組み込み手続き（`Write`/`WriteLn`/`Read`/`ReadLn`）は今回のスコープでも
    /// 個々のシグネチャに対する引数の型・数のチェックは行わない（引数式自体の
    /// 内部エラーの検出のみ）。ユーザー定義の`PROCEDURE`はシグネチャ
    /// （引数の型・個数・`VAR`引数）を検査する。
    ///
    /// 手続き呼び出しは文であり値を持たない。識別子が`FUNCTION`に解決された
    /// 場合は、その戻り値が捨てられてしまう不正な呼び出しとしてエラーにする
    /// （関数呼び出しは式としてのみ評価できる）。
    fn check_proc_call(&mut self, name: &Identifier, args: &[ast::Expr]) {
        const BUILTIN_PROCEDURES: &[&str] = &["write", "writeln", "read", "readln"];
        if BUILTIN_PROCEDURES.contains(&name.name.to_ascii_lowercase().as_str()) {
            for arg in args {
                self.infer_expr_type(arg);
            }
            return;
        }

        match self.symbol_table.lookup(&name.name).cloned() {
            Some(info) => match info.kind {
                SymbolKind::Proc { params } => self.check_call_args(name, &params, args),
                SymbolKind::Func { .. } => {
                    self.diagnostics.push(Diagnostic::new(
                        name.span,
                        Severity::Error,
                        format!(
                            "'{}' is a function; a function cannot be called as a statement (its return value would be discarded)",
                            name.name
                        ),
                    ));
                    for arg in args {
                        self.infer_expr_type(arg);
                    }
                }
                SymbolKind::Var | SymbolKind::Const | SymbolKind::Param { .. } => {
                    self.diagnostics.push(Diagnostic::new(
                        name.span,
                        Severity::Error,
                        format!("'{}' is not a procedure", name.name),
                    ));
                    for arg in args {
                        self.infer_expr_type(arg);
                    }
                }
            },
            None => {
                self.diagnostics.push(Diagnostic::new(
                    name.span,
                    Severity::Error,
                    format!("Undefined procedure '{}'", name.name),
                ));
                for arg in args {
                    self.infer_expr_type(arg);
                }
            }
        }
    }

    /// 関数呼び出し式の型検査。戻り値の型を返す。
    fn check_func_call(&mut self, name: &Identifier, args: &[ast::Expr]) -> Type {
        match self.symbol_table.lookup(&name.name).cloned() {
            Some(info) => match info.kind {
                SymbolKind::Func {
                    params,
                    return_type,
                } => {
                    self.check_call_args(name, &params, args);
                    return_type
                }
                SymbolKind::Proc { .. } => {
                    self.diagnostics.push(Diagnostic::new(
                        name.span,
                        Severity::Error,
                        format!(
                            "'{}' is a procedure; procedures cannot be used in an expression",
                            name.name
                        ),
                    ));
                    for arg in args {
                        self.infer_expr_type(arg);
                    }
                    Type::Error
                }
                SymbolKind::Var | SymbolKind::Const | SymbolKind::Param { .. } => {
                    self.diagnostics.push(Diagnostic::new(
                        name.span,
                        Severity::Error,
                        format!("'{}' is not a function", name.name),
                    ));
                    for arg in args {
                        self.infer_expr_type(arg);
                    }
                    Type::Error
                }
            },
            None => {
                self.diagnostics.push(Diagnostic::new(
                    name.span,
                    Severity::Error,
                    format!("Undefined function '{}'", name.name),
                ));
                for arg in args {
                    self.infer_expr_type(arg);
                }
                Type::Error
            }
        }
    }

    /// 呼び出し（`PROCEDURE`/`FUNCTION`いずれも共通）の実引数を検査する:
    /// 引数の個数、各引数の型（`VAR`引数は同一型のみ、値引数は代入互換で可）、
    /// `VAR`引数には変数（左辺値）しか渡せないこと。
    fn check_call_args(&mut self, callee_name: &Identifier, params: &[ParamSignature], args: &[ast::Expr]) {
        if args.len() != params.len() {
            self.diagnostics.push(Diagnostic::new(
                callee_name.span,
                Severity::Error,
                format!(
                    "'{}' expects {} argument(s), found {}",
                    callee_name.name,
                    params.len(),
                    args.len()
                ),
            ));
            for arg in args {
                self.infer_expr_type(arg);
            }
            return;
        }

        for (i, (arg, param)) in args.iter().zip(params.iter()).enumerate() {
            let arg_ty = self.infer_expr_type(arg);
            // 引数式自体が既にエラー型の場合、それに起因する追加の診断は
            // 出さない（カスケードエラー防止）。
            if arg_ty == Type::Error {
                continue;
            }

            if param.by_ref && !self.is_lvalue(arg) {
                self.diagnostics.push(Diagnostic::new(
                    arg.span(),
                    Severity::Error,
                    "Cannot pass expression as VAR parameter",
                ));
                continue;
            }

            // `VAR`引数は呼び出し元の変数への参照そのものであり、型の
            // 暗黙昇格（`INTEGER`を`REAL`のVAR引数に渡すなど）を許すと
            // 呼び出し先が書き込んだ値の型が変わってしまうため、値引数
            // （代入互換で可）とは異なり厳密な型一致を要求する。
            let compatible = if param.by_ref {
                param.ty == arg_ty
            } else {
                assignment_compatible(param.ty, arg_ty)
            };
            if !compatible {
                self.diagnostics.push(Diagnostic::new(
                    arg.span(),
                    Severity::Error,
                    format!(
                        "Type mismatch: argument {} of '{}' expects '{}', found '{arg_ty}'",
                        i + 1,
                        callee_name.name,
                        param.ty
                    ),
                ));
            }
        }
    }

    /// `VAR`引数に渡せる左辺値（変数・仮引数への参照）かどうか。
    /// リテラルや式（`x + 1`など）、定数、関数呼び出しはすべて左辺値ではない。
    fn is_lvalue(&self, expr: &ast::Expr) -> bool {
        match expr {
            ast::Expr::Identifier(ident) => matches!(
                self.symbol_table.lookup(&ident.name).map(|info| &info.kind),
                Some(SymbolKind::Var) | Some(SymbolKind::Param { .. })
            ),
            _ => false,
        }
    }

    // ---- 式の型推論 ----

    fn infer_expr_type(&mut self, expr: &ast::Expr) -> Type {
        match expr {
            ast::Expr::IntLiteral(..) => Type::Integer,
            ast::Expr::RealLiteral(..) => Type::Real,
            ast::Expr::BoolLiteral(..) => Type::Boolean,
            ast::Expr::StringLiteral(value, span) => self.infer_string_literal_type(value, *span),
            ast::Expr::Identifier(ident) => self.infer_identifier_type(ident),
            ast::Expr::BinaryOp { op, lhs, rhs, span } => {
                let lhs_ty = self.infer_expr_type(lhs);
                let rhs_ty = self.infer_expr_type(rhs);
                self.infer_binary_op_type(*op, lhs_ty, rhs_ty, *span)
            }
            ast::Expr::UnaryOp { op, operand, span } => {
                let operand_ty = self.infer_expr_type(operand);
                self.infer_unary_op_type(*op, operand_ty, *span)
            }
            ast::Expr::Paren(inner, _) => self.infer_expr_type(inner),
            ast::Expr::FuncCall { name, args, .. } => self.check_func_call(name, args),
            // `Expr`は`#[non_exhaustive]`。配列添字式・集合式など今後追加される
            // バリアントは、追加時にここを拡張するまでの間`Type::Error`とする。
            _ => Type::Error,
        }
    }

    /// 識別子1つだけの式の型推論。
    ///
    /// `VAR`/`CONST`/仮引数はその宣言された型をそのまま返す。
    ///
    /// `FUNCTION`の名前が括弧なしで（`Expr::FuncCall`ではなく`Expr::Identifier`
    /// として）現れた場合は、引数なしの関数呼び出しと解釈する
    /// （伝統的なPascalでは引数のない関数呼び出しの括弧を省略できる）。
    /// このとき対象の関数が実際には引数を取る場合はエラーになる
    /// （`Foo`という記述だけでは引数を渡しようがないため）。
    ///
    /// `PROCEDURE`の名前が式中に現れた場合はエラー（手続きは値を持たない）。
    fn infer_identifier_type(&mut self, ident: &Identifier) -> Type {
        match self.symbol_table.lookup(&ident.name).cloned() {
            Some(info) => match info.kind {
                SymbolKind::Var | SymbolKind::Const | SymbolKind::Param { .. } => info.ty,
                SymbolKind::Func {
                    params,
                    return_type,
                } => {
                    if params.is_empty() {
                        return_type
                    } else {
                        self.diagnostics.push(Diagnostic::new(
                            ident.span,
                            Severity::Error,
                            format!(
                                "Function '{}' expects {} argument(s), found 0",
                                ident.name,
                                params.len()
                            ),
                        ));
                        Type::Error
                    }
                }
                SymbolKind::Proc { .. } => {
                    self.diagnostics.push(Diagnostic::new(
                        ident.span,
                        Severity::Error,
                        format!(
                            "'{}' is a procedure; procedures cannot be used in an expression",
                            ident.name
                        ),
                    ));
                    Type::Error
                }
            },
            None => {
                self.diagnostics.push(Diagnostic::new(
                    ident.span,
                    Severity::Error,
                    format!("Undefined identifier '{}'", ident.name),
                ));
                Type::Error
            }
        }
    }

    /// 文字列リテラルの型推論。
    ///
    /// # 方針: 長さ1の文字列リテラルのみ`CHAR`として扱う
    ///
    /// 現在の`wasd-ast`には正式な`STRING`型が存在しない
    /// （`ast::TypeExpr`はINTEGER/REAL/BOOLEAN/CHARのみ）。ISO 7185の
    /// Pascalでも文字列リテラルは本来`PACKED ARRAY [1..n] OF CHAR`型の
    /// 値であり、配列型が未実装の現時点でその型を正しく表現することは
    /// できない。一方で、長さ1の文字列リテラル（`'x'`）はISO 7185上も
    /// `CHAR`型の値として直接使えるため、この場合に限り`CHAR`として
    /// 受理する。
    ///
    /// 長さ0または2文字以上の文字列リテラルは、対応する型（配列型/
    /// `STRING[n]`型）が未実装であることを`Severity::Warning`の診断で
    /// 知らせたうえで（今回のスコープでは複数文字の文字列を扱えないのは
    /// 既知の制約であり、まだ実装していない機能を使おうとしたことを表す
    /// のであって、書いたプログラム自体の意味エラーではないため
    /// `Error`ではなく`Warning`とする）、`Type::Error`を返す。これにより
    /// この式を使った以降の演算・代入で無関係なカスケードエラーが
    /// 出るのを防ぐ。配列型/`STRING[n]`型を追加する際にここを見直すこと。
    fn infer_string_literal_type(&mut self, value: &str, span: Span) -> Type {
        if value.chars().count() == 1 {
            Type::Char
        } else {
            self.diagnostics.push(Diagnostic::new(
                span,
                Severity::Warning,
                "string literals with a length other than 1 are not supported yet \
                 (STRING/array types are not implemented); this literal is ignored \
                 for type checking purposes",
            ));
            Type::Error
        }
    }

    fn infer_literal_type(&mut self, literal: &ast::Literal) -> Type {
        match literal {
            ast::Literal::Int(..) => Type::Integer,
            ast::Literal::Real(..) => Type::Real,
            ast::Literal::Bool(..) => Type::Boolean,
            ast::Literal::Str(value, span) => self.infer_string_literal_type(value, *span),
        }
    }

    fn infer_binary_op_type(&mut self, op: ast::BinOp, lhs: Type, rhs: Type, span: Span) -> Type {
        // どちらかが既にエラー型なら、演算自体に対する追加の診断は出さない
        // （カスケードエラー防止）。
        if lhs == Type::Error || rhs == Type::Error {
            return Type::Error;
        }

        use ast::BinOp::*;
        match op {
            Add | Sub | Mul => match numeric_result(lhs, rhs) {
                Some(ty) => ty,
                None => self.binary_type_error(op, lhs, rhs, span),
            },
            // `/`は標準Pascalの実数除算演算子であり、オペランドが両方
            // INTEGERであっても結果はREALになる（DIVとは異なる）。
            Div => match numeric_result(lhs, rhs) {
                Some(_) => Type::Real,
                None => self.binary_type_error(op, lhs, rhs, span),
            },
            IntDiv | Mod => {
                if lhs == Type::Integer && rhs == Type::Integer {
                    Type::Integer
                } else {
                    self.binary_type_error(op, lhs, rhs, span)
                }
            }
            Eq | NotEq | Lt | Gt | LtEq | GtEq => {
                if lhs == rhs || numeric_result(lhs, rhs).is_some() {
                    Type::Boolean
                } else {
                    self.binary_type_error(op, lhs, rhs, span)
                }
            }
            And | Or => {
                if lhs == Type::Boolean && rhs == Type::Boolean {
                    Type::Boolean
                } else {
                    self.binary_type_error(op, lhs, rhs, span)
                }
            }
        }
    }

    fn binary_type_error(&mut self, op: ast::BinOp, lhs: Type, rhs: Type, span: Span) -> Type {
        self.diagnostics.push(Diagnostic::new(
            span,
            Severity::Error,
            format!(
                "Type mismatch: cannot apply '{}' to '{lhs}' and '{rhs}'",
                bin_op_symbol(op)
            ),
        ));
        Type::Error
    }

    fn infer_unary_op_type(&mut self, op: ast::UnOp, operand: Type, span: Span) -> Type {
        if operand == Type::Error {
            return Type::Error;
        }

        match op {
            ast::UnOp::Neg => match operand {
                Type::Integer | Type::Real => operand,
                _ => {
                    self.diagnostics.push(Diagnostic::new(
                        span,
                        Severity::Error,
                        format!("Type mismatch: cannot apply unary '-' to '{operand}'"),
                    ));
                    Type::Error
                }
            },
            ast::UnOp::Not => match operand {
                Type::Boolean => Type::Boolean,
                _ => {
                    self.diagnostics.push(Diagnostic::new(
                        span,
                        Severity::Error,
                        format!("Type mismatch: cannot apply 'NOT' to '{operand}'"),
                    ));
                    Type::Error
                }
            },
        }
    }
}

/// `INTEGER`/`REAL`同士の二項演算の結果型。`INTEGER op REAL`
/// （どちらの順序でも）は`REAL`への暗黙昇格を許可する。
/// どちらかが`INTEGER`/`REAL`以外なら`None`。
fn numeric_result(lhs: Type, rhs: Type) -> Option<Type> {
    match (lhs, rhs) {
        (Type::Integer, Type::Integer) => Some(Type::Integer),
        (Type::Real, Type::Real) | (Type::Real, Type::Integer) | (Type::Integer, Type::Real) => {
            Some(Type::Real)
        }
        _ => None,
    }
}

/// 代入`target := value`が型検査上許可されるかどうか。
///
/// 完全一致に加え、`REAL := INTEGER`の暗黙昇格のみを許可する
/// （`INTEGER := REAL`のような縮小変換は不可）。どちらかが`Type::Error`の
/// 場合はカスケードエラー防止のため許可扱いにする。
fn assignment_compatible(target: Type, value: Type) -> bool {
    if target == Type::Error || value == Type::Error {
        return true;
    }
    target == value || (target == Type::Real && value == Type::Integer)
}

fn bin_op_symbol(op: ast::BinOp) -> &'static str {
    use ast::BinOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        IntDiv => "DIV",
        Mod => "MOD",
        Eq => "=",
        NotEq => "<>",
        Lt => "<",
        Gt => ">",
        LtEq => "<=",
        GtEq => ">=",
        And => "AND",
        Or => "OR",
    }
}

fn type_from_type_expr(ty: &ast::TypeExpr) -> Type {
    match ty {
        ast::TypeExpr::Integer(_) => Type::Integer,
        ast::TypeExpr::Real(_) => Type::Real,
        ast::TypeExpr::Boolean(_) => Type::Boolean,
        ast::TypeExpr::Char(_) => Type::Char,
        // `TypeExpr`は`#[non_exhaustive]`。配列型/レコード型などが将来
        // 追加された際は、ここを拡張するまでの間`Type::Error`とする。
        _ => Type::Error,
    }
}

/// `CASE`ラベルの重複検出用キー。型の種類ごとに接頭辞を分けることで、
/// 異なる型のリテラル同士（例えば整数の`1`と実数の`1.0`）を誤って
/// 同一視しないようにする。
fn case_label_key(label: &ast::Literal) -> String {
    match label {
        ast::Literal::Int(v, _) => format!("int:{v}"),
        ast::Literal::Real(v, _) => format!("real:{v}"),
        ast::Literal::Str(v, _) => format!("str:{v}"),
        ast::Literal::Bool(v, _) => format!("bool:{v}"),
    }
}

/// 診断メッセージ用に`CASE`ラベルを人間が読める形の文字列にする。
fn describe_case_label(label: &ast::Literal) -> String {
    match label {
        ast::Literal::Int(v, _) => v.to_string(),
        ast::Literal::Real(v, _) => v.to_string(),
        ast::Literal::Str(v, _) => format!("'{v}'"),
        ast::Literal::Bool(v, _) => v.to_string().to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ソース文字列を`wasd-lexer`→`wasd-parser`→`wasd-sema`の順に通し、
    /// 型検査の診断だけを取り出す統合テスト用ヘルパー。字句解析・構文解析
    /// 自体のエラーはこのテストスイートの対象外なので、両方とも空である
    /// ことをアサートしてから型検査に進む。
    fn check(source: &str) -> Vec<Diagnostic> {
        let (tokens, lex_diags) = wasd_lexer::Lexer::new(source).tokenize();
        assert!(
            lex_diags.is_empty(),
            "unexpected lexer diagnostics for {source:?}: {lex_diags:?}"
        );
        let (program, parse_diags) = wasd_parser::Parser::new(tokens).parse_program();
        assert!(
            parse_diags.is_empty(),
            "unexpected parser diagnostics for {source:?}: {parse_diags:?}"
        );
        let program = program.expect("should parse a Program");
        SemaContext::new().check_program(&program)
    }

    fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
        diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect()
    }

    /// テスト対象(1): 正しく型付けされたプログラムでエラーが出ないこと。
    #[test]
    fn well_typed_program_has_no_diagnostics() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x := 1 + 2 END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// テスト対象(2): 型不一致の代入でエラーが出ること。
    #[test]
    fn assignment_type_mismatch_is_reported() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x := TRUE END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// テスト対象(3): INTEGERとREALの混在演算で暗黙昇格が正しく働くこと。
    #[test]
    fn integer_and_real_mix_promotes_to_real() {
        let diags = check("PROGRAM Foo; VAR x: REAL; BEGIN x := 1 + 2.5 END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// `INTEGER := REAL`のような縮小変換は許可されないこと。
    #[test]
    fn narrowing_assignment_from_real_to_integer_is_rejected() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; y: REAL; BEGIN x := y END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// テスト対象(4): IF/WHILEの条件がBOOLEANでない場合にエラーが出ること。
    #[test]
    fn if_condition_must_be_boolean() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN IF x THEN x := 1 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("Condition must be of type BOOLEAN"));
    }

    #[test]
    fn while_condition_must_be_boolean() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN WHILE x DO x := x + 1 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("Condition must be of type BOOLEAN"));
    }

    /// テスト対象(5): 未定義識別子の参照でエラーが出ること。
    #[test]
    fn undefined_identifier_is_reported() {
        let diags = check("PROGRAM Foo; BEGIN y := 1 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Undefined identifier 'y'"));
    }

    /// テスト対象(6): DIV/MODにREALを使うとエラーになること。
    #[test]
    fn div_rejects_real_operand() {
        let diags = check("PROGRAM Foo; VAR x: REAL; BEGIN WriteLn(x DIV 2) END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("DIV"));
    }

    #[test]
    fn mod_rejects_real_operand() {
        let diags = check("PROGRAM Foo; VAR x: REAL; BEGIN WriteLn(x MOD 2) END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("MOD"));
    }

    /// `/`はオペランドが両方INTEGERでも結果がREALになること。
    #[test]
    fn real_division_of_two_integers_yields_real() {
        let diags = check("PROGRAM Foo; VAR x: REAL; BEGIN x := 4 / 2 END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// テスト対象(7): 1つの型エラーの後、カスケードエラーが過剰に
    /// 発生しないこと(`Type::Error`伝播の検証)。
    ///
    /// `TRUE + 1`が型エラーで`Type::Error`を返すため、それを`x`
    /// (INTEGER)に代入する際に追加の「代入の型不一致」エラーは
    /// 出ないはず。
    #[test]
    fn type_error_does_not_cascade_into_assignment_check() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x := TRUE + 1 END.");
        let errs = errors(&diags);
        assert_eq!(
            errs.len(),
            1,
            "expected exactly one diagnostic (no cascade), got: {diags:?}"
        );
        assert!(errs[0].message.contains("Type mismatch: cannot apply '+'"));
    }

    /// テスト対象(8): 複数の独立した型エラーを含むプログラムで、
    /// それら全てが1回の解析で報告されること(エラー耐性の検証)。
    #[test]
    fn reports_multiple_independent_errors_in_one_pass() {
        let diags =
            check("PROGRAM Foo; VAR x: INTEGER; BEGIN x := TRUE; IF x THEN x := 1; y := 2 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 3, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
        assert!(errs[1]
            .message
            .contains("Condition must be of type BOOLEAN"));
        assert!(errs[2].message.contains("Undefined identifier 'y'"));
    }

    /// `CONST`宣言の型がリテラルから正しく推論されること。
    #[test]
    fn const_decl_type_is_inferred_from_literal() {
        let diags = check("PROGRAM Foo; CONST Pi = 3.14; VAR x: REAL; BEGIN x := Pi END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 単一文字の文字列リテラルは`CHAR`として扱われること。
    #[test]
    fn single_char_string_literal_is_treated_as_char() {
        let diags = check("PROGRAM Foo; VAR c: CHAR; BEGIN c := 'x' END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 複数文字の文字列リテラルは(今回未対応のため)警告に留まり、
    /// エラーにはならないこと。
    #[test]
    fn multi_char_string_literal_produces_only_a_warning() {
        let diags = check("PROGRAM Foo; VAR c: CHAR; BEGIN c := 'xy' END.");
        assert!(errors(&diags).is_empty(), "unexpected errors: {diags:?}");
        assert_eq!(diags.len(), 1, "diagnostics: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Warning);
    }

    /// 識別子の参照はcase-insensitiveに解決されること。
    #[test]
    fn identifier_lookup_is_case_insensitive() {
        let diags = check("PROGRAM Foo; VAR Count: INTEGER; BEGIN count := 1 END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 単項演算子の型検査。
    #[test]
    fn unary_not_requires_boolean_operand() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN IF NOT x THEN x := 1 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("NOT"));
    }

    /// 未知の識別子への手続き呼び出しはエラーになること。
    #[test]
    fn calling_an_unknown_procedure_is_an_error() {
        let diags = check("PROGRAM Foo; BEGIN Frobnicate(1) END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Undefined procedure 'Frobnicate'"));
    }

    // ---- PROCEDURE/FUNCTION ----

    /// 正しく宣言・呼び出しされたPROCEDUREでエラーが出ないこと。
    #[test]
    fn well_typed_procedure_call_has_no_diagnostics() {
        let diags = check(
            r#"
            PROGRAM Foo;
            PROCEDURE Inc(VAR x: INTEGER);
            BEGIN
                x := x + 1
            END;
            VAR y: INTEGER;
            BEGIN
                y := 1;
                Inc(y)
            END.
            "#,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 正しく宣言・呼び出しされたFUNCTIONでエラーが出ず、戻り値の型が
    /// 正しく代入に使えること。
    #[test]
    fn well_typed_function_call_has_no_diagnostics() {
        let diags = check(
            r#"
            PROGRAM Foo;
            FUNCTION Square(x: INTEGER): INTEGER;
            BEGIN
                Square := x * x
            END;
            VAR y: INTEGER;
            BEGIN
                y := Square(3)
            END.
            "#,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 再帰呼び出し（階乗）が型エラーなく解析できること。
    #[test]
    fn recursive_function_call_is_allowed() {
        let diags = check(
            r#"
            PROGRAM Foo;
            FUNCTION Fact(n: INTEGER): INTEGER;
            BEGIN
                IF n <= 1 THEN
                    Fact := 1
                ELSE
                    Fact := n * Fact(n - 1)
            END;
            VAR r: INTEGER;
            BEGIN
                r := Fact(5)
            END.
            "#,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// `VAR`引数にリテラル（左辺値でない式）を渡すとエラーになること。
    #[test]
    fn passing_literal_to_var_param_is_rejected() {
        let diags = check(
            r#"
            PROGRAM Foo;
            PROCEDURE Inc(VAR x: INTEGER);
            BEGIN
                x := x + 1
            END;
            BEGIN
                Inc(1)
            END.
            "#,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("Cannot pass expression as VAR parameter"));
    }

    /// `VAR`引数に式（`x + 1`）を渡すとエラーになること。
    #[test]
    fn passing_expression_to_var_param_is_rejected() {
        let diags = check(
            r#"
            PROGRAM Foo;
            PROCEDURE Inc(VAR x: INTEGER);
            BEGIN
                x := x + 1
            END;
            VAR y: INTEGER;
            BEGIN
                Inc(y + 1)
            END.
            "#,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("Cannot pass expression as VAR parameter"));
    }

    /// 引数の個数が合わない呼び出しはエラーになること。
    #[test]
    fn calling_with_wrong_argument_count_is_an_error() {
        let diags = check(
            r#"
            PROGRAM Foo;
            FUNCTION Add(a, b: INTEGER): INTEGER;
            BEGIN
                Add := a + b
            END;
            VAR x: INTEGER;
            BEGIN
                x := Add(1)
            END.
            "#,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("expects 2 argument(s), found 1"));
    }

    /// 引数の型が合わない呼び出しはエラーになること。
    #[test]
    fn calling_with_wrong_argument_type_is_an_error() {
        let diags = check(
            r#"
            PROGRAM Foo;
            FUNCTION Add(a, b: INTEGER): INTEGER;
            BEGIN
                Add := a + b
            END;
            VAR x: INTEGER;
            BEGIN
                x := Add(1, TRUE)
            END.
            "#,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// `FUNCTION`を文として（戻り値を捨てて）呼び出すとエラーになること。
    #[test]
    fn calling_a_function_as_a_statement_is_an_error() {
        let diags = check(
            r#"
            PROGRAM Foo;
            FUNCTION Square(x: INTEGER): INTEGER;
            BEGIN
                Square := x * x
            END;
            BEGIN
                Square(3)
            END.
            "#,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("cannot be called as a statement"));
    }

    /// `PROCEDURE`を式として使おうとするとエラーになること。
    #[test]
    fn using_a_procedure_in_an_expression_is_an_error() {
        let diags = check(
            r#"
            PROGRAM Foo;
            PROCEDURE DoNothing;
            BEGIN
            END;
            VAR x: INTEGER;
            BEGIN
                x := DoNothing
            END.
            "#,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("procedures cannot be used in an expression"));
    }

    /// 関数本体の外から関数名に代入しようとするとエラーになること。
    #[test]
    fn assigning_to_a_function_name_outside_its_body_is_an_error() {
        let diags = check(
            r#"
            PROGRAM Foo;
            FUNCTION Square(x: INTEGER): INTEGER;
            BEGIN
                Square := x * x
            END;
            BEGIN
                Square := 1
            END.
            "#,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("Cannot assign to function 'Square' outside of its own body"));
    }

    /// 関数本体内での戻り値の型不一致はエラーになること。
    #[test]
    fn return_value_assignment_type_mismatch_is_reported() {
        let diags = check(
            r#"
            PROGRAM Foo;
            FUNCTION IsPositive(x: INTEGER): BOOLEAN;
            BEGIN
                IsPositive := 1
            END;
            BEGIN
            END.
            "#,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// `PROCEDURE`名への代入はエラーになること。
    #[test]
    fn assigning_to_a_procedure_name_is_an_error() {
        let diags = check(
            r#"
            PROGRAM Foo;
            PROCEDURE DoNothing;
            BEGIN
            END;
            BEGIN
                DoNothing := 1
            END.
            "#,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("Cannot assign to procedure 'DoNothing'"));
    }

    /// ローカル変数（仮引数含む）が外側スコープの同名変数を隠す
    /// （シャドーイング）こと。これはエラーにならず、関数内では
    /// ローカルの型が使われる。
    #[test]
    fn local_parameter_shadows_outer_variable() {
        let diags = check(
            r#"
            PROGRAM Foo;
            VAR x: BOOLEAN;
            FUNCTION Double(x: INTEGER): INTEGER;
            BEGIN
                Double := x * 2
            END;
            VAR y: INTEGER;
            BEGIN
                y := Double(21)
            END.
            "#,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 引数なしの関数呼び出し（括弧省略）が正しく解決されること。
    #[test]
    fn niladic_function_call_without_parens_is_resolved() {
        let diags = check(
            r#"
            PROGRAM Foo;
            FUNCTION GetAnswer: INTEGER;
            BEGIN
                GetAnswer := 42
            END;
            VAR x: INTEGER;
            BEGIN
                x := GetAnswer
            END.
            "#,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 括弧を省略した呼び出しで、実際には引数を要求する関数の場合はエラーに
    /// なること。
    #[test]
    fn omitting_parens_for_a_function_that_requires_arguments_is_an_error() {
        let diags = check(
            r#"
            PROGRAM Foo;
            FUNCTION Square(x: INTEGER): INTEGER;
            BEGIN
                Square := x * x
            END;
            VAR y: INTEGER;
            BEGIN
                y := Square
            END.
            "#,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("expects 1 argument(s), found 0"));
    }

    /// 変数を手続きとして呼び出そうとするとエラーになること。
    #[test]
    fn calling_a_variable_as_a_procedure_is_an_error() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x(1) END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("'x' is not a procedure"));
    }

    /// 未定義の関数呼び出しはエラーになること。
    #[test]
    fn calling_an_unknown_function_is_an_error() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x := Frobnicate(1) END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Undefined function 'Frobnicate'"));
    }

    // ---- FOR ----

    /// 正しく書かれたFOR文でエラーが出ないこと。
    #[test]
    fn well_typed_for_statement_has_no_diagnostics() {
        let diags = check(
            "PROGRAM Foo; VAR i, sum: INTEGER; BEGIN sum := 0; FOR i := 1 TO 10 DO sum := sum + i END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// `DOWNTO`でも正しく書かれていればエラーが出ないこと。
    #[test]
    fn well_typed_for_downto_statement_has_no_diagnostics() {
        let diags = check(
            "PROGRAM Foo; VAR i, sum: INTEGER; BEGIN sum := 0; FOR i := 10 DOWNTO 1 DO sum := sum + i END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// ループ変数がREALだとエラーになること。
    #[test]
    fn for_loop_variable_must_be_integer() {
        let diags = check("PROGRAM Foo; VAR i: REAL; BEGIN FOR i := 1 TO 10 DO i := i END.");
        let errs = errors(&diags);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("FOR loop variable") && e.message.contains("INTEGER")),
            "diagnostics: {diags:?}"
        );
    }

    /// `start`式がBOOLEANだとエラーになること。
    #[test]
    fn for_loop_start_must_be_integer() {
        let diags =
            check("PROGRAM Foo; VAR i: INTEGER; BEGIN FOR i := TRUE TO 10 DO i := i END.");
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|e| e.message.contains("start value")),
            "diagnostics: {diags:?}"
        );
    }

    /// `end`式がBOOLEANだとエラーになること。
    #[test]
    fn for_loop_end_must_be_integer() {
        let diags =
            check("PROGRAM Foo; VAR i: INTEGER; BEGIN FOR i := 1 TO TRUE DO i := i END.");
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|e| e.message.contains("end value")),
            "diagnostics: {diags:?}"
        );
    }

    /// ループ本体でループ変数に代入するとエラーになること
    /// （ISO 7185 6.8.3.9: ループ変数の変更はerroneous）。
    #[test]
    fn assigning_to_the_for_loop_variable_in_its_body_is_an_error() {
        let diags = check("PROGRAM Foo; VAR i: INTEGER; BEGIN FOR i := 1 TO 10 DO i := i + 1 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("Cannot assign to FOR loop control variable 'i'"));
    }

    /// ループ本体を抜けた後は、同名変数への代入が再び許可されること
    /// （ループ変数の禁止はループ本体内に限定される）。
    #[test]
    fn assigning_to_the_loop_variable_after_the_loop_is_allowed() {
        let diags = check(
            "PROGRAM Foo; VAR i: INTEGER; BEGIN FOR i := 1 TO 10 DO i := i; i := 0 END.",
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("Cannot assign to FOR loop control variable 'i'"));
    }

    /// 未定義の識別子をループ変数に使うとエラーになること。
    #[test]
    fn undefined_for_loop_variable_is_reported() {
        let diags = check("PROGRAM Foo; BEGIN FOR i := 1 TO 10 DO i := i END.");
        let errs = errors(&diags);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Undefined identifier 'i'")),
            "diagnostics: {diags:?}"
        );
    }

    // ---- REPEAT UNTIL ----

    /// 正しく書かれたREPEAT UNTIL文でエラーが出ないこと。
    #[test]
    fn well_typed_repeat_statement_has_no_diagnostics() {
        let diags = check(
            "PROGRAM Foo; VAR i: INTEGER; BEGIN i := 0; REPEAT i := i + 1 UNTIL i = 10 END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// REPEAT本体が複数文でもエラーが出ないこと。
    #[test]
    fn well_typed_repeat_with_multiple_statements_has_no_diagnostics() {
        let diags = check(
            r#"
            PROGRAM Foo;
            VAR i, sum: INTEGER;
            BEGIN
                i := 0;
                sum := 0;
                REPEAT
                    sum := sum + i;
                    i := i + 1
                UNTIL i >= 10
            END.
            "#,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// `UNTIL`の条件がBOOLEANでない場合にエラーが出ること。
    #[test]
    fn repeat_until_condition_must_be_boolean() {
        let diags = check("PROGRAM Foo; VAR i: INTEGER; BEGIN REPEAT i := i + 1 UNTIL i END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("Condition must be of type BOOLEAN"));
    }

    /// REPEAT本体内の型エラーも検出されること。
    #[test]
    fn type_errors_inside_repeat_body_are_reported() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN REPEAT x := TRUE UNTIL x = 1 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    // ---- CASE ----

    /// 正しく書かれたCASE文でエラーが出ないこと。
    #[test]
    fn well_typed_case_statement_has_no_diagnostics() {
        let diags = check(
            r#"
            PROGRAM Foo;
            VAR x, y: INTEGER;
            BEGIN
                x := 2;
                CASE x OF
                    1, 2: y := 1;
                    3: y := 2
                END
            END.
            "#,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// CHARセレクタとCHARラベル（単一文字リテラル）でもエラーが出ないこと。
    #[test]
    fn well_typed_case_statement_with_char_selector_has_no_diagnostics() {
        let diags = check(
            r#"
            PROGRAM Foo;
            VAR c: CHAR; y: INTEGER;
            BEGIN
                c := 'a';
                CASE c OF
                    'a': y := 1;
                    'b': y := 2
                END
            END.
            "#,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// セレクタがREALだとエラーになること（順序型ではないため）。
    #[test]
    fn case_selector_must_be_ordinal_type() {
        let diags = check(
            "PROGRAM Foo; VAR x: REAL; y: INTEGER; BEGIN CASE x OF 1: y := 1 END END.",
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("ordinal type"));
    }

    /// ラベルの型がセレクタの型と一致しない場合エラーになること。
    #[test]
    fn case_label_type_must_match_selector_type() {
        let diags = check(
            "PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF TRUE: y := 1 END END.",
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// 同じ値のラベルが複数の分岐に重複して出現するとエラーになること。
    #[test]
    fn duplicate_case_labels_are_rejected() {
        let diags = check(
            "PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF 1: y := 1; 1: y := 2 END END.",
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Duplicate CASE label"));
    }

    /// 同じ分岐内でのラベルの重複（`1, 1: ...`）もエラーになること。
    #[test]
    fn duplicate_case_labels_within_the_same_branch_are_rejected() {
        let diags =
            check("PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF 1, 1: y := 1 END END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Duplicate CASE label"));
    }

    /// 各分岐の本体の型エラーも検出されること。
    #[test]
    fn type_errors_inside_case_branch_body_are_reported() {
        let diags =
            check("PROGRAM Foo; VAR x: INTEGER; BEGIN CASE x OF 1: x := TRUE END END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// 分岐に該当しない値が来うる（非網羅的な）CASE文でもエラー・警告が
    /// 出ないこと（方針: 非網羅性は診断しない）。
    #[test]
    fn non_exhaustive_case_statement_is_not_diagnosed() {
        let diags =
            check("PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF 1: y := 1 END END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }
}
