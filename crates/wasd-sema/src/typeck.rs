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

use crate::symbol_table::{SymbolInfo, SymbolKind, SymbolTable};
use crate::types::Type;

/// 意味解析（今回のスコープでは型検査のみ）の実行コンテキスト。
pub struct SemaContext {
    symbol_table: SymbolTable,
    diagnostics: Vec<Diagnostic>,
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
        }
    }

    /// `Program`を走査し、型検査を行う。
    ///
    /// 1パス目で`VAR`/`CONST`宣言をシンボルテーブルに登録し、2パス目で
    /// `Block`内の文を順に型検査する。パニックせず、エラーがあっても
    /// 可能な限り走査を継続してすべての`Diagnostic`を蓄積する。
    pub fn check_program(&mut self, program: &ast::Program) -> Vec<Diagnostic> {
        // 同じ`SemaContext`を使い回しても前回の解析結果を引きずらないよう、
        // 呼び出しのたびに状態をリセットする。
        self.symbol_table = SymbolTable::new();
        self.diagnostics = Vec::new();

        self.collect_const_decls(&program.const_decls);
        self.collect_var_decls(&program.var_decls);
        self.check_block(&program.body);

        std::mem::take(&mut self.diagnostics)
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
            ast::Statement::Compound(block) => self.check_block(block),
            ast::Statement::ProcCall { name, args, .. } => {
                self.check_proc_call(name, args);
            }
            // `Statement`は`#[non_exhaustive]`。`FOR`/`REPEAT UNTIL`/`CASE`など
            // 今後追加されるバリアントは、追加時にこの型検査を拡張するまでの
            // 間、ここでは何もしない（このステップのスコープ外のため）。
            _ => {}
        }
    }

    fn check_assignment(&mut self, target: &Identifier, value: &ast::Expr) {
        let value_ty = self.infer_expr_type(value);
        match self.symbol_table.lookup(&target.name) {
            Some(info) => {
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
    /// 今回のスコープでは`PROCEDURE`宣言自体が無いため、組み込み手続き
    /// （`Write`/`WriteLn`/`Read`/`ReadLn`）のみ暫定的にサポートする。
    /// それ以外の識別子は未定義の手続きとしてエラーにする。引数の型検査は
    /// 「引数式自体の内部エラー（未定義識別子など）を検出する」程度に留め、
    /// 個々の組み込み手続きのシグネチャに対する引数の型・数のチェックは
    /// 行わない。
    fn check_proc_call(&mut self, name: &Identifier, args: &[ast::Expr]) {
        const BUILTIN_PROCEDURES: &[&str] = &["write", "writeln", "read", "readln"];
        let is_builtin = BUILTIN_PROCEDURES.contains(&name.name.to_ascii_lowercase().as_str());
        if !is_builtin {
            self.diagnostics.push(Diagnostic::new(
                name.span,
                Severity::Error,
                format!("Undefined procedure '{}'", name.name),
            ));
        }
        for arg in args {
            self.infer_expr_type(arg);
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
            // `Expr`は`#[non_exhaustive]`。配列添字式・集合式など今後追加される
            // バリアントは、追加時にここを拡張するまでの間`Type::Error`とする。
            _ => Type::Error,
        }
    }

    fn infer_identifier_type(&mut self, ident: &Identifier) -> Type {
        match self.symbol_table.lookup(&ident.name) {
            Some(info) => info.ty,
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
}
