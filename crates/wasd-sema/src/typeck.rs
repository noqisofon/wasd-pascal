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

use std::collections::HashMap;

use wasd_ast as ast;
use wasd_ast::{Diagnostic, Dialect, Identifier, Severity, Span};

use crate::dialect_check;
use crate::symbol_table::{ParamSignature, SymbolInfo, SymbolKind, SymbolTable};
use crate::types::{ArrayType, Type};

/// レコード型のフィールド一覧。`crate::types::Type::Record`のドキュメント
/// 参照: `Type::Record`自体は名前（識別用のハンドル）のみを保持し、実際の
/// フィールド一覧はここに切り出す。これにより`next: ^Node`のような
/// 自己参照的なレコード定義を、`Type`自体を無限サイズにすることなく
/// 表現できる。
#[derive(Debug, Clone)]
struct RecordInfo {
    fields: Vec<RecordField>,
}

#[derive(Debug, Clone)]
struct RecordField {
    /// ソース上の表記のまま（フィールド名はcase-insensitiveに比較する。
    /// `SymbolTable`と同じ方針）。
    name: String,
    ty: Type,
}

/// 意味解析（型検査 + dialectチェック）の実行コンテキスト。
pub struct SemaContext {
    /// 現在有効なdialect。`Dialect::Iso7185`が既定で、UCSD拡張構文
    /// （`UNIT`/`USES`/`OTHERWISE`/`STRING[n]`/16進数リテラル/
    /// コンパイラディレクティブ）は`Dialect::Ucsd`が明示的に指定された
    /// 場合のみ許可される。`check_program`/`check_unit`をまたいでも
    /// 変わらない（コンストラクタでのみ設定する）ため、両メソッドの
    /// 冒頭のリセット処理では触らない。
    dialect: Dialect,
    symbol_table: SymbolTable,
    diagnostics: Vec<Diagnostic>,
    /// `TYPE`セクションで宣言された型名(小文字正規化済み) -> 解決済み`Type`。
    /// `TypeExpr::Named`の解決に使う。レコード型については、対応する
    /// `record_registry`エントリの名前と同じ文字列を持つ
    /// `Type::Record(name)`を値として持つ（`crate::types::Type`の
    /// ドキュメント参照）。
    type_table: HashMap<String, Type>,
    /// レコード型の識別名(小文字正規化済み) -> フィールド一覧。
    /// `Type::Record`のドキュメント参照。
    record_registry: HashMap<String, RecordInfo>,
    /// `TYPE`宣言を経ない無名`RECORD`型に合成名を割り当てるための連番。
    next_anon_id: u32,
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
        Self::new(Dialect::default())
    }
}

impl SemaContext {
    pub fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            symbol_table: SymbolTable::new(),
            diagnostics: Vec::new(),
            type_table: HashMap::new(),
            record_registry: HashMap::new(),
            next_anon_id: 0,
            current_function: None,
            for_loop_vars: Vec::new(),
        }
    }

    /// UCSD拡張構文が現在の`dialect`で許可されているかを判定し、許可されて
    /// いなければ`Diagnostic`を積む。判定ロジック自体は[`dialect_check`]
    /// モジュールに切り出してある（単体テストしやすくするため）。
    ///
    /// この関数はエラーを報告するだけで、呼び出し元の走査を止めない。
    /// 呼び出し元は診断の有無に関わらずASTの残りの意味解析を継続すること
    /// （LSPで他のエラーも同時に見せるため。Step 3のドキュメント参照）。
    fn check_dialect_gate(&mut self, span: Span, feature: &str, required: Dialect) {
        if let Some(diag) = dialect_check::check_dialect_gate(self.dialect, span, feature, required)
        {
            self.diagnostics.push(diag);
        }
    }

    /// `wasd_ast::TypeExpr`（ソース上に書かれた型注釈）を型検査用の
    /// `Type`へ変換する。`TypeExpr::StringN`はUCSD拡張なので、ここで
    /// dialectチェックを行う（`self`を要求するのはこのため。以前は
    /// フリー関数だったが、dialectチェックのために`SemaContext`のメソッドに
    /// 変更した）。配列・レコード・ポインタ型はISO 7185相当の標準機能
    /// なのでdialectチェックの対象外。
    fn type_from_type_expr(&mut self, ty: &ast::TypeExpr) -> Type {
        match ty {
            ast::TypeExpr::Integer(_) => Type::Integer,
            ast::TypeExpr::Real(_) => Type::Real,
            ast::TypeExpr::Boolean(_) => Type::Boolean,
            ast::TypeExpr::Char(_) => Type::Char,
            ast::TypeExpr::StringN(n, span) => {
                self.check_dialect_gate(*span, "STRING[n] type", Dialect::Ucsd);
                Type::StringN(*n)
            }
            ast::TypeExpr::Named(ident) => self.resolve_named_type(ident),
            ast::TypeExpr::Array {
                index_type,
                element_type,
                packed,
                ..
            } => self.resolve_array_type(index_type, element_type, *packed),
            ast::TypeExpr::Record { fields, .. } => self.resolve_record_type(None, fields),
            ast::TypeExpr::Pointer(inner, _) => {
                let pointee = self.type_from_type_expr(inner);
                Type::Pointer(Box::new(pointee))
            }
            // `TypeExpr::Subrange`は`Array`の添字位置以外には現れない
            // （`wasd-parser`は`Array`の`[low..high]`の中でのみ`Subrange`を
            // 構築する）。単独で型の位置に現れることは想定していないが、
            // 万一渡された場合でも`Type::Integer`にフォールバックして
            // パニックを避ける。`TypeExpr`は`#[non_exhaustive]`なので、
            // 将来追加されるバリアントもここで`Type::Error`にフォールバック
            // させるまでのワイルドカードを兼ねる。
            ast::TypeExpr::Subrange { .. } => Type::Integer,
            #[allow(unreachable_patterns)]
            _ => Type::Error,
        }
    }

    /// `TypeExpr::Named`（`TYPE`セクションで宣言された型名への参照）の解決。
    /// 見つからない場合は「未知の型」の診断を出し`Type::Error`を返す。
    fn resolve_named_type(&mut self, ident: &Identifier) -> Type {
        let key = ident.name.to_ascii_lowercase();
        if let Some(ty) = self.type_table.get(&key) {
            return ty.clone();
        }
        self.diagnostics.push(Diagnostic::new(
            ident.span,
            Severity::Error,
            format!("Unknown type '{}'", ident.name),
        ));
        Type::Error
    }

    /// `ARRAY [index_type] OF element_type`の解決。今回のスコープでは
    /// 添字は`INTEGER`のサブレンジ（コンパイル時定数のリテラル2つ）のみを
    /// サポートする（タスク文書参照）。
    fn resolve_array_type(
        &mut self,
        index_type: &ast::TypeExpr,
        element_type: &ast::TypeExpr,
        packed: bool,
    ) -> Type {
        let (low, high) = match index_type {
            ast::TypeExpr::Subrange { low, high, .. } => self.eval_subrange_bounds(low, high),
            other => {
                self.diagnostics.push(Diagnostic::new(
                    other.span(),
                    Severity::Error,
                    "array index type must be an INTEGER subrange (e.g. 1..10); this \
                     implementation does not support any other index type yet",
                ));
                (0, -1)
            }
        };
        let element = self.type_from_type_expr(element_type);
        Type::Array(Box::new(ArrayType {
            low,
            high,
            element,
            packed,
        }))
    }

    /// 配列添字のサブレンジ境界`low..high`を評価する。今回のスコープでは
    /// `INTEGER`リテラルのみをサポートする（タスク文書参照: 「添字の型が
    /// index_typeと適合すること。今回はINTEGERのサブレンジのみ対応」）。
    fn eval_subrange_bounds(&mut self, low: &ast::Literal, high: &ast::Literal) -> (i64, i64) {
        let low_span = low.span();
        let high_span = high.span();
        let low_val = match low {
            ast::Literal::Int(v, _) => *v,
            _ => {
                self.diagnostics.push(Diagnostic::new(
                    low_span,
                    Severity::Error,
                    "array index bounds must be INTEGER literals",
                ));
                0
            }
        };
        let high_val = match high {
            ast::Literal::Int(v, _) => *v,
            _ => {
                self.diagnostics.push(Diagnostic::new(
                    high_span,
                    Severity::Error,
                    "array index bounds must be INTEGER literals",
                ));
                0
            }
        };
        if low_val > high_val {
            self.diagnostics.push(Diagnostic::new(
                Span::new(low_span.start, high_span.end),
                Severity::Error,
                format!(
                    "array index low bound {low_val} must not exceed the high bound {high_val}"
                ),
            ));
        }
        (low_val, high_val)
    }

    /// `RECORD field1, field2: T1; ... END`の解決。`explicit_name`が`Some`
    /// なら`TYPE`セクションでの宣言由来（識別名はその型名そのもの）、
    /// `None`なら`VAR`/仮引数/フィールドの型注釈に直接書かれた無名の
    /// `RECORD`（識別名は連番から合成する）。
    ///
    /// # 設計判断: レコードの型同一性は名前的（nominal）に判定する
    ///
    /// ISO 7185自体はレコード型の型同一性を構造的に定義しておらず、
    /// 実装依存の余地がある（タスク文書参照）。ここでは「同じ`TYPE`宣言に
    /// 由来するレコードのみ代入可能」という単純な名前的型付けを採用する。
    /// そのため無名`RECORD`（`TYPE`宣言を経ないもの）は、たとえ
    /// フィールド構成が完全に同一でも、書かれた箇所が異なれば別の型として
    /// 扱われる（`next_anon_id`で連番の合成名を割り当てるため）。
    fn resolve_record_type(
        &mut self,
        explicit_name: Option<&str>,
        fields: &[ast::FieldDecl],
    ) -> Type {
        let identity = match explicit_name {
            Some(name) => name.to_string(),
            None => {
                self.next_anon_id += 1;
                format!(
                    "{}{}",
                    crate::types::ANONYMOUS_RECORD_PREFIX,
                    self.next_anon_id
                )
            }
        };
        let key = identity.to_ascii_lowercase();
        self.resolve_record_fields_into(&key, fields);
        Type::Record(identity)
    }

    /// [`Self::resolve_record_type`]の下請け。フィールド一覧を解決し、
    /// `record_registry[key]`へ（既存のプレースホルダエントリがあれば
    /// それを上書きする形で）書き込む。`TYPE`セクションの前方参照解決
    /// （`collect_type_decls`のドキュメント参照）で、レコード名を先に
    /// 仮登録してからフィールドだけを後で解決する際にも使う。
    fn resolve_record_fields_into(&mut self, key: &str, fields: &[ast::FieldDecl]) {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut resolved_fields = Vec::new();
        for field_decl in fields {
            let field_ty = self.type_from_type_expr(&field_decl.ty);
            for name in &field_decl.names {
                let lower = name.name.to_ascii_lowercase();
                if !seen.insert(lower) {
                    self.diagnostics.push(Diagnostic::new(
                        name.span,
                        Severity::Error,
                        format!("field '{}' is already declared in this RECORD", name.name),
                    ));
                    continue;
                }
                resolved_fields.push(RecordField {
                    name: name.name.clone(),
                    ty: field_ty.clone(),
                });
            }
        }

        self.record_registry.insert(
            key.to_string(),
            RecordInfo {
                fields: resolved_fields,
            },
        );
    }

    /// `TYPE`セクションを2パスで解決する。
    ///
    /// # 前方参照の解決方針: レコード型を指すポインタ型のみを対象とする
    ///
    /// `TYPE PNode = ^Node; Node = RECORD next: PNode; ... END;`のように、
    /// ポインタ型が同じ`TYPE`セクション内で後から宣言されるレコード型を
    /// 指すことを許可する（連結リストの定番パターン）。これを実現するため
    /// 2パスに分ける:
    ///
    /// - パス0: このセクション内の`RECORD`型宣言をすべて先に見つけ、
    ///   `record_registry`/`type_table`へ**名前だけ**を仮登録する
    ///   （フィールド一覧はまだ空）。レコード型の識別（`Type::Record`）は
    ///   名前だけで完結する（`crate::types::Type`のドキュメント参照）ため、
    ///   フィールドが未解決でも「この名前はレコード型である」という事実
    ///   だけで、それを指すポインタ型は解決できる。
    /// - パス1: 宣言順に各`TypeDecl`を解決する。`RECORD`型はパス0で
    ///   仮登録済みのフィールドを実際の内容で上書きし、それ以外
    ///   （配列・ポインタ・`STRING[n]`・型名の別名など）は宣言順に
    ///   逐次解決する。
    ///
    /// # 既知の制限: レコード以外への前方参照は未対応
    ///
    /// パス0はレコード型の宣言だけを先読みするため、`TYPE PArr = ^MyArr;
    /// MyArr = ARRAY [1..5] OF INTEGER;`のような「レコード以外の型を指す
    /// ポインタの前方参照」や、`TYPE A = B; B = INTEGER;`のような
    /// 「非ポインタの前方参照（別名の別名）」は解決できず、「Unknown
    /// type」エラーになる。ISO 7185はポインタ型の前方参照を型全般に
    /// 許可しているため、これは意図的なスコープ制限である（タスク文書が
    /// 明示的に要求するのは連結リストパターン、すなわちレコードへの
    /// ポインタの前方参照のみ）。
    fn collect_type_decls(&mut self, decls: &[ast::TypeDecl]) {
        // 重複宣言をあらかじめ洗い出しておく。以降のパス0/パス1は
        // 重複した（=先頭以外の）宣言を完全に無視することで、重複宣言の
        // 中身で正規の宣言を上書きしてしまう事故を防ぐ。
        let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut is_duplicate = vec![false; decls.len()];
        for (i, decl) in decls.iter().enumerate() {
            let key = decl.name.name.to_ascii_lowercase();
            if !seen_names.insert(key) {
                self.diagnostics.push(Diagnostic::new(
                    decl.name.span,
                    Severity::Error,
                    format!("Type '{}' is already declared", decl.name.name),
                ));
                is_duplicate[i] = true;
            }
        }

        // パス0: レコード型の名前だけを先に仮登録する（前方参照解決の
        // ドキュメント参照）。
        for (i, decl) in decls.iter().enumerate() {
            if is_duplicate[i] {
                continue;
            }
            if let ast::TypeExpr::Record { .. } = &decl.ty {
                let key = decl.name.name.to_ascii_lowercase();
                self.record_registry
                    .insert(key.clone(), RecordInfo { fields: Vec::new() });
                self.type_table
                    .insert(key, Type::Record(decl.name.name.clone()));
            }
        }

        // パス1: 宣言順に解決する。
        for (i, decl) in decls.iter().enumerate() {
            if is_duplicate[i] {
                continue;
            }
            let key = decl.name.name.to_ascii_lowercase();
            let resolved = match &decl.ty {
                ast::TypeExpr::Record { fields, .. } => {
                    self.resolve_record_fields_into(&key, fields);
                    Type::Record(decl.name.name.clone())
                }
                other => self.type_from_type_expr(other),
            };
            self.type_table.insert(key, resolved);
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
        self.type_table = HashMap::new();
        self.record_registry = HashMap::new();
        self.next_anon_id = 0;
        self.current_function = None;
        self.for_loop_vars = Vec::new();

        self.check_uses_clause(&program.uses);

        self.collect_type_decls(&program.type_decls);
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

    /// `Unit`を走査し、dialectチェック + 型検査を行う。
    ///
    /// # 今回のスコープ: `INTERFACE`/`IMPLEMENTATION`間の突き合わせは行わない
    ///
    /// `INTERFACE`部の`PROCEDURE`/`FUNCTION`シグネチャ（`proc_signatures`/
    /// `func_signatures`）はシンボルテーブルへ登録しない。`IMPLEMENTATION`部
    /// の完全な宣言（`proc_decls`/`func_decls`）が対応するシグネチャを
    /// 持つかどうかの検証や、`INTERFACE`部の宣言と`IMPLEMENTATION`部の
    /// 宣言が同名の場合の整合性チェックは、`USES`によるクロスファイル・
    /// クロスUNITなシンボル解決と合わせて次のステップに切り出す
    /// （`wasd_ast::Unit`のドキュメント参照）。今回はUNIT自体・`USES`節に
    /// 対するdialectチェックと、`INTERFACE`部の`CONST`/`VAR`宣言、
    /// `IMPLEMENTATION`部の`PROCEDURE`/`FUNCTION`本体の型検査のみを行う。
    pub fn check_unit(&mut self, unit: &ast::Unit) -> Vec<Diagnostic> {
        self.symbol_table = SymbolTable::new();
        self.diagnostics = Vec::new();
        self.type_table = HashMap::new();
        self.record_registry = HashMap::new();
        self.next_anon_id = 0;
        self.current_function = None;
        self.for_loop_vars = Vec::new();

        self.check_dialect_gate(unit.span, "UNIT declarations", Dialect::Ucsd);
        self.check_uses_clause(&unit.interface.uses);

        self.collect_type_decls(&unit.interface.type_decls);
        self.collect_const_decls(&unit.interface.const_decls);
        self.collect_var_decls(&unit.interface.var_decls);

        for proc in &unit.implementation.proc_decls {
            self.check_proc_decl(proc);
        }
        for func in &unit.implementation.func_decls {
            self.check_func_decl(func);
        }

        std::mem::take(&mut self.diagnostics)
    }

    /// UCSD拡張: `USES`節のdialectチェック。空であれば何もしない
    /// （`USES`節自体が存在しなければ、そもそもdialectを問う対象がない）。
    fn check_uses_clause(&mut self, uses: &[Identifier]) {
        if let (Some(first), Some(last)) = (uses.first(), uses.last()) {
            let span = Span::new(first.span.start, last.span.end);
            self.check_dialect_gate(span, "USES clauses", Dialect::Ucsd);
        }
    }

    // ---- PROCEDURE/FUNCTION宣言 ----

    fn param_signatures(&mut self, params: &[ast::ParamDecl]) -> Vec<ParamSignature> {
        params
            .iter()
            .map(|p| ParamSignature {
                ty: self.type_from_type_expr(&p.ty),
                by_ref: p.by_ref,
            })
            .collect()
    }

    fn declare_params(&mut self, params: &[ast::ParamDecl]) {
        for p in params {
            let ty = self.type_from_type_expr(&p.ty);
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
        let return_type = self.type_from_type_expr(&decl.return_type);
        self.declare(
            &decl.name,
            return_type.clone(),
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
            let ty = self.type_from_type_expr(&decl.ty);
            for name in &decl.names {
                self.declare(name, ty.clone(), SymbolKind::Var, decl.span);
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
            ast::Statement::Repeat {
                body, until_cond, ..
            } => {
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
                otherwise,
                ..
            } => {
                self.check_case_statement(selector, branches, otherwise);
            }
            ast::Statement::Compound(block) => self.check_block(block),
            ast::Statement::ProcCall { name, args, .. } => {
                self.check_proc_call(name, args);
            }
            ast::Statement::CompilerDirective { name, span, .. } => {
                self.check_compiler_directive(name, *span);
            }
            // `Statement`は`#[non_exhaustive]`。将来追加されるバリアントは、
            // 追加時にこの型検査を拡張するまでの間、ここでは何もしない
            // （このステップのスコープ外のため）。
            _ => {}
        }
    }

    /// UCSD拡張: コンパイラディレクティブ `(*$I foo.pas*)`のdialectチェック。
    ///
    /// # CONFIRMED: `$I`と`$R2`/`$R4`
    ///
    /// `$I`（include。引数にファイル名を取る）はUCSD PascalとBorland Pascal
    /// に共通する既知のディレクティブとして確認できた。加えて2026-09-01の
    /// 一次資料調査（リポジトリのUCSD Pascal一次資料調査メモ参照）により、
    /// SofTech Microsystems, *UCSD p-System and UCSD Pascal Version IV:
    /// Internal Architecture Guide* (First edition, March 1981)から`$R2`/
    /// `$R4`（`realsize`を32/64bitに設定するディレクティブ）の存在も確認
    /// できたため、これらもここでは既知のディレクティブとして扱う。
    ///
    /// `name`にはレキサー（`wasd_lexer::scan_compiler_directive`）が`$`直後の
    /// 連続した英数字を貪欲に取った結果が入る（`wasd_lexer::TokenKind::
    /// CompilerDirective`のドキュメント参照）ため、`$R2`/`$R4`は`name`が
    /// `"R"`ではなく`"R2"`/`"R4"`そのものになる点に注意。実際に`realsize`を
    /// 切り替えたりファイルをincludeしたりする処理自体は今回のスコープ外
    /// （`wasd_ast::Statement::CompilerDirective`のドキュメント参照）。
    ///
    /// # UNCONFIRMED: `$I`/`$R2`/`$R4`以外のディレクティブ
    ///
    /// `$U`/`$S`等、トグル式の他のディレクティブがUCSD Pascalに存在した
    /// 可能性はあるが、一次資料での確認は取れていない。既知のディレクティブ
    /// として断定せず、未知のディレクティブはエラーではなく警告に留める。
    fn check_compiler_directive(&mut self, name: &str, span: Span) {
        self.check_dialect_gate(span, "compiler directives ((*$...*))", Dialect::Ucsd);

        let known = name.eq_ignore_ascii_case("I")
            || name.eq_ignore_ascii_case("R2")
            || name.eq_ignore_ascii_case("R4");
        if !known {
            self.diagnostics.push(Diagnostic::new(
                span,
                Severity::Warning,
                format!(
                    "unknown compiler directive '${name}' (not verified against primary UCSD \
                     Pascal documentation in this implementation; UNCONFIRMED)"
                ),
            ));
        }
    }

    /// `CASE selector OF label1, label2: stmt1; ... [OTHERWISE stmtN] END`の型検査。
    ///
    /// - `selector`は順序型でなければならない。今回のスコープでは
    ///   `INTEGER`/`CHAR`/`BOOLEAN`のみをサポートする（`REAL`は不可）。
    /// - 各`label`の型は`selector`の型と一致しなければならない。
    /// - `label`はすべての分岐を通じて重複してはならない。
    /// - UCSD拡張の`otherwise`が`Some`の場合、`Dialect::Ucsd`が要求される
    ///   （`Iso7185`では使用不可）。dialectエラーを報告した後も`otherwise`の
    ///   本体自体は型検査を継続する。
    ///
    /// # 方針: 非網羅性（どの分岐にも一致しない値が来た場合）は診断しない
    ///
    /// ISO/IEC 7185:1990はCASE文で選択子がどの`case-constant`にも一致しない
    /// 場合の動作を規定しておらず（実装依存/未定義）、コンパイル時に
    /// 「網羅性」を機械的に判定することも一般には困難である
    /// （`INTEGER`は事実上無限の値域を持つため、静的な網羅性チェックには
    /// 意味がない）。そのため本実装は非網羅性を診断しない
    /// （エラーにも警告にもしない）。UCSD拡張の`OTHERWISE`句を使えば、
    /// 利用者が明示的にフォールバックを書ける。
    fn check_case_statement(
        &mut self,
        selector: &ast::Expr,
        branches: &[ast::CaseBranch],
        otherwise: &Option<Box<ast::Statement>>,
    ) {
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

        if let Some(otherwise_stmt) = otherwise {
            self.check_dialect_gate(otherwise_stmt.span(), "OTHERWISE clause", Dialect::Ucsd);
            self.check_statement(otherwise_stmt);
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

    /// 代入文`target := value`の型検査。`target`は単純な識別子とは限らず、
    /// 配列添字アクセス・レコードフィールドアクセス・ポインタ
    /// デリファレンス（の組み合わせ）にもなり得る
    /// （`wasd_ast::stmt::Statement::Assignment`のドキュメント参照）。
    fn check_assignment(&mut self, target: &ast::Expr, value: &ast::Expr) {
        let value_ty = self.infer_expr_type(value);
        match target {
            ast::Expr::Identifier(ident) => self.check_assignment_to_identifier(ident, value_ty),
            ast::Expr::IndexAccess { .. }
            | ast::Expr::FieldAccess { .. }
            | ast::Expr::Deref { .. } => {
                // 配列要素・レコードフィールド・デリファレンスは、基点となる
                // 式（配列/レコード/ポインタ）さえ解決できれば常に左辺値で
                // あるため、`FOR`ループ変数のような特別扱いは不要
                // （そもそも`FOR`ループ変数として宣言できるのは単純な
                // 識別子のみ）。診断は`infer_expr_type`側（存在しない
                // フィールド・範囲外添字・非ポインタのデリファレンスなど）が
                // 出す。
                let target_ty = self.infer_expr_type(target);
                if !assignment_compatible(&target_ty, &value_ty) {
                    self.diagnostics.push(Diagnostic::new(
                        target.span(),
                        Severity::Error,
                        format!("Type mismatch: cannot assign '{value_ty}' to '{target_ty}'"),
                    ));
                }
            }
            _ => {
                self.diagnostics.push(Diagnostic::new(
                    target.span(),
                    Severity::Error,
                    "left-hand side of assignment is not a variable",
                ));
                self.infer_expr_type(target);
            }
        }
    }

    /// [`Self::check_assignment`]の下請け: 単純な識別子への代入。
    fn check_assignment_to_identifier(&mut self, target: &Identifier, value_ty: Type) {
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
                    if !assignment_compatible(&target_ty, &value_ty) {
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
                        if !assignment_compatible(&return_type, &value_ty) {
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
    /// この意味解析レイヤーでは、`WriteLn`に渡された引数がどの型であっても
    /// （`Type::StringLiteral`を含め）意味解析上のエラーにはならない。
    /// 「`WriteLn`が実際に出力できる型」の制限は、この後段の`wasd-pcode`の
    /// コード生成が担う（`crates/wasd-pcode/src/codegen.rs`の
    /// `gen_writeln_call`参照。現時点では`INTEGER`/`BOOLEAN`/文字列リテラル/
    /// `STRING[n]`変数のみサポート）。そのため`Type::StringLiteral`を
    /// ここで特別扱いする必要はなく、既存の「引数式自体の内部エラーの検出
    /// のみ」という方針の範囲内で自然に受理される。
    ///
    /// 手続き呼び出しは文であり値を持たない。識別子が`FUNCTION`に解決された
    /// 場合は、その戻り値が捨てられてしまう不正な呼び出しとしてエラーにする
    /// （関数呼び出しは式としてのみ評価できる）。
    fn check_proc_call(&mut self, name: &Identifier, args: &[ast::Expr]) {
        const BUILTIN_PROCEDURES: &[&str] = &["write", "writeln", "read", "readln"];
        let lower_name = name.name.to_ascii_lowercase();
        if BUILTIN_PROCEDURES.contains(&lower_name.as_str()) {
            for arg in args {
                self.infer_expr_type(arg);
            }
            return;
        }

        // 組み込み手続き`NEW`/`DISPOSE`: 引数は1つで、ポインタ型でなければ
        // ならない。今回のスコープでは意味解析レベルでこのチェックのみを
        // 行う（実際のヒープ確保・解放はp-code生成/ランタイムの課題。
        // タスク文書参照）。
        if lower_name == "new" || lower_name == "dispose" {
            self.check_new_or_dispose_call(name, args);
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

    /// `NEW(p)`/`DISPOSE(p)`の型検査。引数はちょうど1つで、その型が
    /// ポインタ型でなければならない（タスク文書参照: 「引数がポインタ型
    /// であることをチェックするだけでよい」）。
    fn check_new_or_dispose_call(&mut self, name: &Identifier, args: &[ast::Expr]) {
        if args.len() != 1 {
            self.diagnostics.push(Diagnostic::new(
                name.span,
                Severity::Error,
                format!("'{}' expects 1 argument, found {}", name.name, args.len()),
            ));
            for arg in args {
                self.infer_expr_type(arg);
            }
            return;
        }

        let arg_ty = self.infer_expr_type(&args[0]);
        if arg_ty != Type::Error && !matches!(arg_ty, Type::Pointer(_)) {
            self.diagnostics.push(Diagnostic::new(
                args[0].span(),
                Severity::Error,
                format!(
                    "'{}' expects a pointer-typed argument, found '{arg_ty}'",
                    name.name
                ),
            ));
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
    fn check_call_args(
        &mut self,
        callee_name: &Identifier,
        params: &[ParamSignature],
        args: &[ast::Expr],
    ) {
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
                assignment_compatible(&param.ty, &arg_ty)
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
    ///
    /// 配列添字アクセス・レコードフィールドアクセスは、基点となる式が
    /// 左辺値である場合に限り左辺値になる（`arr[i]`は`arr`が変数/仮引数の
    /// ときのみ、`rec.field`は`rec`が変数/仮引数のときのみ）。ポインタの
    /// デリファレンス（`p^`）は、`p`自体の左辺値性に関わらず常に左辺値
    /// （ヒープ上の領域を指しており、常にアドレス指定可能なため）。
    fn is_lvalue(&self, expr: &ast::Expr) -> bool {
        match expr {
            ast::Expr::Identifier(ident) => matches!(
                self.symbol_table.lookup(&ident.name).map(|info| &info.kind),
                Some(SymbolKind::Var) | Some(SymbolKind::Param { .. })
            ),
            ast::Expr::IndexAccess { array, .. } => self.is_lvalue(array),
            ast::Expr::FieldAccess { record, .. } => self.is_lvalue(record),
            ast::Expr::Deref { .. } => true,
            _ => false,
        }
    }

    // ---- 式の型推論 ----

    fn infer_expr_type(&mut self, expr: &ast::Expr) -> Type {
        match expr {
            ast::Expr::IntLiteral(..) => Type::Integer,
            ast::Expr::HexIntLiteral(_, span) => {
                // UCSD拡張: `$FF`。値そのものは通常のINTEGERと同じ意味を
                // 持つため、dialectエラーを報告した後も型はINTEGERのまま
                // 扱う（以降の演算・代入の型検査を継続できるようにするため）。
                self.check_dialect_gate(*span, "hexadecimal literals ($FF)", Dialect::Ucsd);
                Type::Integer
            }
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
            ast::Expr::NilLiteral(_) => Type::Nil,
            ast::Expr::IndexAccess { array, index, .. } => {
                self.infer_index_access_type(array, index)
            }
            ast::Expr::FieldAccess { record, field, .. } => {
                self.infer_field_access_type(record, field)
            }
            ast::Expr::Deref { pointer, .. } => self.infer_deref_type(pointer),
            // `Expr`は`#[non_exhaustive]`。集合式など今後追加される
            // バリアントは、追加時にここを拡張するまでの間`Type::Error`とする。
            _ => Type::Error,
        }
    }

    /// 配列添字アクセス`array[index]`の型推論。要素型を返す。
    ///
    /// - `index`の型は`INTEGER`でなければならない（今回のスコープでは
    ///   添字型は`INTEGER`のサブレンジのみ対応。タスク文書参照）。
    /// - `index`がコンパイル時定数（リテラル、あるいは単項`-`を付けた
    ///   リテラル）として評価できる場合のみ、添字の範囲チェック
    ///   （`Array::low..=high`に収まっているか）を行う。実行時の範囲
    ///   チェックは今回のscopeでは行わない（タスク文書参照）。
    fn infer_index_access_type(&mut self, array: &ast::Expr, index: &ast::Expr) -> Type {
        let array_ty = self.infer_expr_type(array);
        let index_ty = self.infer_expr_type(index);

        match &array_ty {
            Type::Array(arr) => {
                if index_ty != Type::Integer && index_ty != Type::Error {
                    self.diagnostics.push(Diagnostic::new(
                        index.span(),
                        Severity::Error,
                        format!("array index must be of type INTEGER, found '{index_ty}'"),
                    ));
                } else if let Some(value) = const_eval_int(index) {
                    if value < arr.low || value > arr.high {
                        self.diagnostics.push(Diagnostic::new(
                            index.span(),
                            Severity::Error,
                            format!(
                                "array index {value} is out of bounds [{}..{}]",
                                arr.low, arr.high
                            ),
                        ));
                    }
                }
                arr.element.clone()
            }
            Type::Error => Type::Error,
            other => {
                self.diagnostics.push(Diagnostic::new(
                    array.span(),
                    Severity::Error,
                    format!("cannot index into non-array type '{other}'"),
                ));
                Type::Error
            }
        }
    }

    /// レコードフィールドアクセス`record.field`の型推論。
    fn infer_field_access_type(&mut self, record: &ast::Expr, field: &Identifier) -> Type {
        let record_ty = self.infer_expr_type(record);
        match &record_ty {
            Type::Record(name) => {
                let key = name.to_ascii_lowercase();
                let lower_field = field.name.to_ascii_lowercase();
                let found = self.record_registry.get(&key).and_then(|info| {
                    info.fields
                        .iter()
                        .find(|f| f.name.to_ascii_lowercase() == lower_field)
                });
                match found {
                    Some(f) => f.ty.clone(),
                    None => {
                        self.diagnostics.push(Diagnostic::new(
                            field.span,
                            Severity::Error,
                            format!("record type '{record_ty}' has no field '{}'", field.name),
                        ));
                        Type::Error
                    }
                }
            }
            Type::Error => Type::Error,
            other => {
                self.diagnostics.push(Diagnostic::new(
                    record.span(),
                    Severity::Error,
                    format!("cannot access field of non-record type '{other}'"),
                ));
                Type::Error
            }
        }
    }

    /// ポインタデリファレンス`pointer^`の型推論。指す先の型を返す。
    fn infer_deref_type(&mut self, pointer: &ast::Expr) -> Type {
        let ptr_ty = self.infer_expr_type(pointer);
        match ptr_ty {
            Type::Pointer(pointee) => *pointee,
            Type::Error => Type::Error,
            Type::Nil => {
                self.diagnostics.push(Diagnostic::new(
                    pointer.span(),
                    Severity::Error,
                    "cannot dereference NIL",
                ));
                Type::Error
            }
            other => {
                self.diagnostics.push(Diagnostic::new(
                    pointer.span(),
                    Severity::Error,
                    format!("cannot dereference non-pointer type '{other}'"),
                ));
                Type::Error
            }
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
    /// 長さ1の文字列リテラル（`'x'`）はISO 7185上も`CHAR`型の値として
    /// 直接使えるため、dialectに関わらずこの場合は`CHAR`として受理する。
    ///
    /// # 長さ1以外: dialectを問わず`Type::StringLiteral(n)`
    ///
    /// 長さ`n`（`n != 1`）の文字列リテラルは常に`Type::StringLiteral(n)`
    /// として受理する（dialectによる警告・エラーは出さない）。
    ///
    /// # Step 16: `Type::StringLiteral`は正規の`STRING[n]`型検査と連携する
    ///
    /// `Type::StringLiteral`は文字列リテラル自身の型であり、`STRING[n]`の
    /// ように宣言された最大長を持たない点で`Type::StringN`とは異なる
    /// （`crate::types::Type::StringLiteral`のドキュメント参照）が、Step 15
    /// 時点とは異なり、もはや`WriteLn`への直接渡し専用の「最小対応」では
    /// ない。`Type::StringLiteral(len)`から`Type::StringN(n)`への代入は
    /// `len <= n`であれば[`assignment_compatible`]が許可する（Step 16で
    /// 追加）。それ以外の用途（二項演算子のオペランド等）は引き続き
    /// 通常の型エラーになる。
    fn infer_string_literal_type(&mut self, value: &str, _span: Span) -> Type {
        let len = value.chars().count();
        if len == 1 {
            Type::Char
        } else {
            Type::StringLiteral(len)
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
            Add | Sub | Mul => match numeric_result(&lhs, &rhs) {
                Some(ty) => ty,
                None => self.binary_type_error(op, &lhs, &rhs, span),
            },
            // `/`は標準Pascalの実数除算演算子であり、オペランドが両方
            // INTEGERであっても結果はREALになる（DIVとは異なる）。
            Div => match numeric_result(&lhs, &rhs) {
                Some(_) => Type::Real,
                None => self.binary_type_error(op, &lhs, &rhs, span),
            },
            IntDiv | Mod => {
                if lhs == Type::Integer && rhs == Type::Integer {
                    Type::Integer
                } else {
                    self.binary_type_error(op, &lhs, &rhs, span)
                }
            }
            // `=`/`<>`は、スカラー型同士（下記`Lt`等と共通）に加え、
            // ポインタ型・`NIL`の組み合わせも許可する
            // （タスク文書: 「NILとの比較は任意のポインタ型に対して許可する」）。
            // `ARRAY`/`RECORD`型はISO 7185上も比較演算子の対象ではないため、
            // `is_comparable_scalar`の対象外のまま（下記`pointer_eq_compatible`
            // にも該当しないため`binary_type_error`になる）。
            Eq | NotEq => {
                if pointer_eq_compatible(op, &lhs, &rhs)
                    || (is_comparable_scalar(&lhs)
                        && is_comparable_scalar(&rhs)
                        && (lhs == rhs || numeric_result(&lhs, &rhs).is_some()))
                {
                    Type::Boolean
                } else {
                    self.binary_type_error(op, &lhs, &rhs, span)
                }
            }
            // 大小比較はISO 7185上も順序型（INTEGER/REAL/BOOLEAN/CHAR）と
            // `STRING[n]`のみが対象で、`ARRAY`/`RECORD`/ポインタ型は対象外。
            Lt | Gt | LtEq | GtEq => {
                if is_comparable_scalar(&lhs)
                    && is_comparable_scalar(&rhs)
                    && (lhs == rhs || numeric_result(&lhs, &rhs).is_some())
                {
                    Type::Boolean
                } else {
                    self.binary_type_error(op, &lhs, &rhs, span)
                }
            }
            And | Or => {
                if lhs == Type::Boolean && rhs == Type::Boolean {
                    Type::Boolean
                } else {
                    self.binary_type_error(op, &lhs, &rhs, span)
                }
            }
        }
    }

    fn binary_type_error(&mut self, op: ast::BinOp, lhs: &Type, rhs: &Type, span: Span) -> Type {
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
fn numeric_result(lhs: &Type, rhs: &Type) -> Option<Type> {
    match (lhs, rhs) {
        (Type::Integer, Type::Integer) => Some(Type::Integer),
        (Type::Real, Type::Real) | (Type::Real, Type::Integer) | (Type::Integer, Type::Real) => {
            Some(Type::Real)
        }
        _ => None,
    }
}

/// 大小比較演算子（`=`/`<>`を含む）の対象になり得るスカラー型かどうか。
/// `ARRAY`/`RECORD`/ポインタ型・`NIL`はISO 7185上も比較演算子の一般的な
/// 対象ではないため対象外（ポインタ型の`=`/`<>`は`NIL`比較・同型ポインタ
/// 同士の比較のみ別途[`pointer_eq_compatible`]で許可する）。
fn is_comparable_scalar(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Integer | Type::Real | Type::Boolean | Type::Char | Type::StringN(_)
    )
}

/// `=`/`<>`に限り、ポインタ型・`NIL`の組み合わせを許可する
/// （タスク文書: 「NILとの比較は任意のポインタ型に対して許可する」）。
/// 同型ポインタ同士の比較（`p1 = p2`）も、`NIL`との比較と同様にここで
/// 扱う（ISO 7185はポインタの大小比較を定義しないため`Lt`等は対象外）。
fn pointer_eq_compatible(op: ast::BinOp, lhs: &Type, rhs: &Type) -> bool {
    if !matches!(op, ast::BinOp::Eq | ast::BinOp::NotEq) {
        return false;
    }
    match (lhs, rhs) {
        (Type::Pointer(_), Type::Pointer(_)) => lhs == rhs,
        (Type::Pointer(_), Type::Nil) | (Type::Nil, Type::Pointer(_)) | (Type::Nil, Type::Nil) => {
            true
        }
        _ => false,
    }
}

/// 代入`target := value`が型検査上許可されるかどうか。
///
/// 完全一致に加え、`REAL := INTEGER`の暗黙昇格、任意の`Type::Pointer`への
/// `NIL`の代入、および文字列リテラルから収まる長さの`STRING[n]`への代入
/// （Step 16で追加。下記参照）を許可する。どちらかが`Type::Error`の場合は
/// カスケードエラー防止のため許可扱いにする。
///
/// # Step 16: `Type::StringLiteral(len) -> Type::StringN(n)`（`len <= n`）
///
/// `s := 'Hello, world!';`のような、`STRING[n]`変数への文字列リテラル
/// 代入を成立させるための特別扱い。リテラルの実際の文字数`len`が宣言された
/// 最大長`n`を超える場合は許可しない（型エラーになる）。
fn assignment_compatible(target: &Type, value: &Type) -> bool {
    if *target == Type::Error || *value == Type::Error {
        return true;
    }
    target == value
        || (*target == Type::Real && *value == Type::Integer)
        || (matches!(target, Type::Pointer(_)) && *value == Type::Nil)
        || matches!(
            (target, value),
            (Type::StringN(n), Type::StringLiteral(len)) if *len <= *n as usize
        )
}

/// 添字式がコンパイル時定数の`INTEGER`リテラルとして評価できる場合に
/// その値を返す。今回のスコープでは配列添字の範囲チェックの対象を
/// 「明らかなリテラル添字」に限定するため（タスク文書参照:
/// 「範囲チェックは今回はコンパイル時定数に対してのみ行う」）、
/// リテラル・単項`-`を付けたリテラル・括弧で囲んだものだけを対象にする
/// （変数を含む式は`None`を返し、範囲チェックの対象外になる）。
fn const_eval_int(expr: &ast::Expr) -> Option<i64> {
    match expr {
        ast::Expr::IntLiteral(v, _) => Some(*v),
        ast::Expr::HexIntLiteral(v, _) => Some(*v),
        ast::Expr::UnaryOp {
            op: ast::UnOp::Neg,
            operand,
            ..
        } => const_eval_int(operand).map(|v| -v),
        ast::Expr::Paren(inner, _) => const_eval_int(inner),
        _ => None,
    }
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
    /// 型検査の診断だけを取り出す統合テスト用ヘルパー（`Dialect::Iso7185`
    /// 固定）。字句解析・構文解析自体のエラーはこのテストスイートの対象外
    /// なので、両方とも空であることをアサートしてから型検査に進む。
    fn check(source: &str) -> Vec<Diagnostic> {
        check_with_dialect(source, Dialect::Iso7185)
    }

    /// [`check`]のdialect指定版。UCSD拡張構文のdialectチェックのテストに使う。
    fn check_with_dialect(source: &str, dialect: Dialect) -> Vec<Diagnostic> {
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
        SemaContext::new(dialect).check_program(&program)
    }

    /// `UNIT`ソース文字列を`wasd-lexer`→`wasd-parser`(`parse_unit`相当)→
    /// `wasd-sema`(`check_unit`)の順に通す統合テスト用ヘルパー。
    fn check_unit_with_dialect(source: &str, dialect: Dialect) -> Vec<Diagnostic> {
        let (tokens, lex_diags) = wasd_lexer::Lexer::new(source).tokenize();
        assert!(
            lex_diags.is_empty(),
            "unexpected lexer diagnostics for {source:?}: {lex_diags:?}"
        );
        let (unit, parse_diags) = wasd_parser::Parser::new(tokens).parse_compilation_unit();
        assert!(
            parse_diags.is_empty(),
            "unexpected parser diagnostics for {source:?}: {parse_diags:?}"
        );
        let unit = match unit.expect("should parse a CompilationUnit") {
            ast::CompilationUnit::Unit(unit) => unit,
            ast::CompilationUnit::Program(_) => panic!("expected a UNIT, got a PROGRAM"),
        };
        SemaContext::new(dialect).check_unit(&unit)
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

    /// Step 15: 複数文字の文字列リテラルを`WriteLn`以外の文脈（ここでは
    /// `CHAR`変数への代入）で使おうとした場合は、引き続き型エラーになる
    /// こと（`Type::StringLiteral`の唯一の用途は`WriteLn`への直接渡しの
    /// みであることのリグレッション確認。以前は`Severity::Warning`のみで
    /// `Type::Error`扱いだったため代入自体はカスケードエラー防止で素通り
    /// していたが、その暫定処理は撤廃した）。
    #[test]
    fn multi_char_string_literal_is_a_type_error_outside_writeln() {
        let diags = check("PROGRAM Foo; VAR c: CHAR; BEGIN c := 'xy' END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
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
        assert!(errs[0].message.contains("cannot be called as a statement"));
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
        let diags = check("PROGRAM Foo; VAR i: INTEGER; BEGIN FOR i := TRUE TO 10 DO i := i END.");
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|e| e.message.contains("start value")),
            "diagnostics: {diags:?}"
        );
    }

    /// `end`式がBOOLEANだとエラーになること。
    #[test]
    fn for_loop_end_must_be_integer() {
        let diags = check("PROGRAM Foo; VAR i: INTEGER; BEGIN FOR i := 1 TO TRUE DO i := i END.");
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
        let diags =
            check("PROGRAM Foo; VAR i: INTEGER; BEGIN FOR i := 1 TO 10 DO i := i; i := 0 END.");
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
        let diags =
            check("PROGRAM Foo; VAR i: INTEGER; BEGIN i := 0; REPEAT i := i + 1 UNTIL i = 10 END.");
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
        let diags =
            check("PROGRAM Foo; VAR x: REAL; y: INTEGER; BEGIN CASE x OF 1: y := 1 END END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("ordinal type"));
    }

    /// ラベルの型がセレクタの型と一致しない場合エラーになること。
    #[test]
    fn case_label_type_must_match_selector_type() {
        let diags = check("PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF TRUE: y := 1 END END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// 同じ値のラベルが複数の分岐に重複して出現するとエラーになること。
    #[test]
    fn duplicate_case_labels_are_rejected() {
        let diags =
            check("PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF 1: y := 1; 1: y := 2 END END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Duplicate CASE label"));
    }

    /// 同じ分岐内でのラベルの重複（`1, 1: ...`）もエラーになること。
    #[test]
    fn duplicate_case_labels_within_the_same_branch_are_rejected() {
        let diags = check("PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF 1, 1: y := 1 END END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Duplicate CASE label"));
    }

    /// 各分岐の本体の型エラーも検出されること。
    #[test]
    fn type_errors_inside_case_branch_body_are_reported() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN CASE x OF 1: x := TRUE END END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// 分岐に該当しない値が来うる（非網羅的な）CASE文でもエラー・警告が
    /// 出ないこと（方針: 非網羅性は診断しない）。
    #[test]
    fn non_exhaustive_case_statement_is_not_diagnosed() {
        let diags = check("PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF 1: y := 1 END END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    // ------------------------------------------------------------------
    // Step 7: dialectチェック
    // ------------------------------------------------------------------

    /// OTHERWISE: `Dialect::Ucsd`では正常に受理される。
    #[test]
    fn otherwise_clause_is_accepted_under_ucsd_dialect() {
        let diags = check_with_dialect(
            "PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF 1: y := 1 OTHERWISE y := 2 END END.",
            Dialect::Ucsd,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// OTHERWISE: 既定の`Dialect::Iso7185`ではdialectエラーになる。
    #[test]
    fn otherwise_clause_is_rejected_under_iso7185_dialect() {
        let diags = check(
            "PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF 1: y := 1 OTHERWISE y := 2 END END.",
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("OTHERWISE"));
        assert!(errs[0].message.contains("UCSD"));
    }

    /// OTHERWISE: dialectエラーが出ても、`OTHERWISE`本体内の無関係な
    /// 型エラーは引き続き検出される（エラー耐性）。
    #[test]
    fn otherwise_clause_dialect_error_does_not_suppress_body_type_errors() {
        let diags = check(
            "PROGRAM Foo; VAR x: INTEGER; BEGIN CASE x OF 1: x := 1 OTHERWISE x := TRUE END END.",
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 2, "diagnostics: {diags:?}");
        assert!(errs.iter().any(|d| d.message.contains("OTHERWISE")));
        assert!(errs.iter().any(|d| d.message.contains("Type mismatch")));
    }

    /// 16進数リテラル: `Dialect::Ucsd`では正常に受理される。
    #[test]
    fn hex_literal_is_accepted_under_ucsd_dialect() {
        let diags = check_with_dialect(
            "PROGRAM Foo; VAR x: INTEGER; BEGIN x := $FF END.",
            Dialect::Ucsd,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 16進数リテラル: 既定の`Dialect::Iso7185`ではdialectエラーになる。
    #[test]
    fn hex_literal_is_rejected_under_iso7185_dialect() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x := $FF END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("UCSD"));
    }

    /// 16進数リテラル: dialectエラーが出ても値はINTEGERとして扱われ続け、
    /// 以降の型検査（この場合は代入自体）はカスケードエラーにならない。
    #[test]
    fn hex_literal_dialect_error_does_not_cascade() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x := $FF + 1 END.");
        let errs = errors(&diags);
        // '$FF'自体のdialectエラー1件のみで、'+'や代入についての追加の
        // 型エラーは出ない（INTEGER + INTEGERとして正しく検査される）。
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("UCSD"));
    }

    /// STRING[n]型: `Dialect::Ucsd`では正常に受理される。
    #[test]
    fn string_n_type_is_accepted_under_ucsd_dialect() {
        let diags = check_with_dialect("PROGRAM Foo; VAR s: STRING[10]; BEGIN END.", Dialect::Ucsd);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// STRING[n]型: 既定の`Dialect::Iso7185`ではdialectエラーになる。
    #[test]
    fn string_n_type_is_rejected_under_iso7185_dialect() {
        let diags = check("PROGRAM Foo; VAR s: STRING[10]; BEGIN END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("STRING"));
        assert!(errs[0].message.contains("UCSD"));
    }

    /// STRING[n]型: dialectエラーが出た後も、プログラム中の他の無関係な
    /// 型エラーは引き続き検出される（エラー耐性）。
    #[test]
    fn string_n_type_dialect_error_does_not_suppress_other_errors() {
        let diags = check("PROGRAM Foo; VAR s: STRING[10]; x: INTEGER; BEGIN x := TRUE END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 2, "diagnostics: {diags:?}");
        assert!(errs.iter().any(|d| d.message.contains("STRING")));
        assert!(errs.iter().any(|d| d.message.contains("Type mismatch")));
    }

    /// Step 16: 文字列リテラルの文字数が`STRING[n]`の`n`ぴったりに収まる
    /// 場合、代入は型エラーにならない（`assignment_compatible`の
    /// `Type::StringLiteral(len) -> Type::StringN(n)`、`len <= n`の規則）。
    #[test]
    fn string_literal_is_assignable_to_a_string_n_variable_when_it_fits() {
        let diags = check_with_dialect(
            "PROGRAM Foo; VAR s: STRING[5]; BEGIN s := 'hello' END.",
            Dialect::Ucsd,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// Step 16: 文字列リテラルの文字数が`STRING[n]`の`n`を超える場合は
    /// 引き続き型エラーになる。
    #[test]
    fn string_literal_longer_than_string_n_max_len_is_a_type_error() {
        let diags = check_with_dialect(
            "PROGRAM Foo; VAR s: STRING[4]; BEGIN s := 'hello' END.",
            Dialect::Ucsd,
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// Step 16: 動作確認用のサンプルプログラム
    /// （`greeting: STRING[80]; greeting := 'Hello, world!'; WriteLn(greeting)`）
    /// が意味解析上エラーなく通ること。
    #[test]
    fn string_test_sample_program_has_no_diagnostics() {
        let diags = check_with_dialect(
            "PROGRAM StringTest; VAR greeting: STRING[80]; BEGIN \
             greeting := 'Hello, world!'; WriteLn(greeting) END.",
            Dialect::Ucsd,
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// Step 15: `WriteLn('hello')`のような、長さ1以外の文字列リテラルを
    /// `WriteLn`へ直接渡すケースは、dialectを問わずエラーにも警告にも
    /// ならない（以前は`Dialect::Iso7185`で`Severity::Warning`を出して
    /// `Type::Error`にしていたが、その暫定処理は撤廃した。このモジュールの
    /// `infer_string_literal_type`ドキュメント参照）。
    #[test]
    fn writeln_with_multi_char_string_literal_has_no_diagnostics_under_either_dialect() {
        let diags = check("PROGRAM Foo; BEGIN WriteLn('hello') END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");

        let diags = check_with_dialect("PROGRAM Foo; BEGIN WriteLn('hello') END.", Dialect::Ucsd);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// USES節: `Dialect::Ucsd`では正常に受理される。
    #[test]
    fn uses_clause_is_accepted_under_ucsd_dialect() {
        let diags = check_with_dialect("PROGRAM Foo; USES Crt; BEGIN END.", Dialect::Ucsd);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// USES節: 既定の`Dialect::Iso7185`ではdialectエラーになる。
    #[test]
    fn uses_clause_is_rejected_under_iso7185_dialect() {
        let diags = check("PROGRAM Foo; USES Crt; BEGIN END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("USES"));
        assert!(errs[0].message.contains("UCSD"));
    }

    /// コンパイラディレクティブ: `Dialect::Ucsd`かつ既知の`$I`であれば
    /// 警告・エラーいずれも出ない。
    #[test]
    fn known_compiler_directive_is_accepted_under_ucsd_dialect_without_warning() {
        let diags = check_with_dialect("PROGRAM Foo; BEGIN (*$I foo.pas*) END.", Dialect::Ucsd);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// コンパイラディレクティブ: 既定の`Dialect::Iso7185`ではdialectエラーに
    /// なる。
    #[test]
    fn compiler_directive_is_rejected_under_iso7185_dialect() {
        let diags = check("PROGRAM Foo; BEGIN (*$I foo.pas*) END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("UCSD"));
    }

    /// コンパイラディレクティブ: `Dialect::Ucsd`かつ既知の`$R2`/`$R4`
    /// （`realsize`設定。Internal Architecture Guideで確認済み）であれば
    /// 警告・エラーいずれも出ない。
    #[test]
    fn known_realsize_directive_is_accepted_under_ucsd_dialect_without_warning() {
        for src in [
            "PROGRAM Foo; BEGIN (*$R2*) END.",
            "PROGRAM Foo; BEGIN (*$R4*) END.",
        ] {
            let diags = check_with_dialect(src, Dialect::Ucsd);
            assert!(
                diags.is_empty(),
                "unexpected diagnostics for {src:?}: {diags:?}"
            );
        }
    }

    /// コンパイラディレクティブ: `Dialect::Ucsd`でも未知のディレクティブ名は
    /// 警告になる（エラーにはしない。UNCONFIRMEDの方針）。
    #[test]
    fn unknown_compiler_directive_is_warned_about_under_ucsd_dialect() {
        let diags = check_with_dialect("PROGRAM Foo; BEGIN (*$Q mystery*) END.", Dialect::Ucsd);
        assert!(errors(&diags).is_empty(), "unexpected errors: {diags:?}");
        assert!(
            diags
                .iter()
                .any(|d| d.severity == Severity::Warning && d.message.contains("$Q")),
            "expected a warning about the unknown directive: {diags:?}"
        );
    }

    /// UNIT: `Dialect::Ucsd`では正常に受理される（INTERFACE/IMPLEMENTATIONの
    /// 宣言も型検査される）。
    #[test]
    fn unit_declaration_is_accepted_under_ucsd_dialect() {
        let src = r#"
            UNIT Greetings;
            INTERFACE
            CONST Greeting = 'H';
            PROCEDURE Hello;
            IMPLEMENTATION
            PROCEDURE Hello;
            VAR x: INTEGER;
            BEGIN
                x := 1
            END;
            END.
        "#;
        let diags = check_unit_with_dialect(src, Dialect::Ucsd);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// UNIT: 既定の`Dialect::Iso7185`ではUNIT自体がdialectエラーになる。
    #[test]
    fn unit_declaration_is_rejected_under_iso7185_dialect() {
        let src = r#"
            UNIT Greetings;
            INTERFACE
            IMPLEMENTATION
            END.
        "#;
        let diags = check_unit_with_dialect(src, Dialect::Iso7185);
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("UNIT"));
        assert!(errs[0].message.contains("UCSD"));
    }

    /// UNIT: `Iso7185`でのUNIT自体のdialectエラーが出ても、
    /// `IMPLEMENTATION`部の無関係な型エラーは引き続き検出される
    /// （エラー耐性）。
    #[test]
    fn unit_dialect_error_does_not_suppress_implementation_type_errors() {
        let src = r#"
            UNIT Greetings;
            INTERFACE
            IMPLEMENTATION
            PROCEDURE Oops;
            VAR x: INTEGER;
            BEGIN
                x := TRUE
            END;
            END.
        "#;
        let diags = check_unit_with_dialect(src, Dialect::Iso7185);
        let errs = errors(&diags);
        assert_eq!(errs.len(), 2, "diagnostics: {diags:?}");
        assert!(errs.iter().any(|d| d.message.contains("UNIT")));
        assert!(errs.iter().any(|d| d.message.contains("Type mismatch")));
    }

    /// UNIT: `USES`節を持つUNITも、既定の`Dialect::Iso7185`では両方
    /// （UNIT自体・USES節）についてdialectエラーが出ること。
    #[test]
    fn unit_with_uses_clause_reports_both_dialect_errors_under_iso7185() {
        let src = r#"
            UNIT Greetings;
            INTERFACE
            USES Crt;
            IMPLEMENTATION
            END.
        "#;
        let diags = check_unit_with_dialect(src, Dialect::Iso7185);
        let errs = errors(&diags);
        assert_eq!(errs.len(), 2, "diagnostics: {diags:?}");
        assert!(errs.iter().any(|d| d.message.contains("UNIT")));
        assert!(errs.iter().any(|d| d.message.contains("USES")));
    }

    /// リグレッション確認: Step 6までのISO 7185相当のテストが、dialectの
    /// 明示指定なし（既定の`Iso7185`）で引き続き全て通ること
    /// （代表例としてPROCEDURE/FUNCTION/FOR/REPEAT/CASEを含むプログラムを
    /// 1本にまとめて検証する）。
    #[test]
    fn iso7185_regression_program_with_procedures_for_repeat_case_has_no_diagnostics() {
        let src = r#"
            PROGRAM Regression;
            VAR total, i: INTEGER;

            FUNCTION Square(n: INTEGER): INTEGER;
            BEGIN
                Square := n * n
            END;

            BEGIN
                total := 0;
                FOR i := 1 TO 5 DO
                    total := total + Square(i);
                REPEAT
                    total := total - 1
                UNTIL total <= 0;
                CASE i OF
                    1, 2: total := 1;
                    3: total := 2
                END
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    // ==================================================================
    // 配列型・レコード型・ポインタ型（Step 9）
    // ==================================================================

    // ---- 配列型 ----

    #[test]
    fn well_typed_array_declaration_and_element_access_has_no_diagnostics() {
        let diags = check(
            "PROGRAM Foo; VAR a: ARRAY [1..10] OF INTEGER; x: INTEGER; \
             BEGIN a[1] := 42; x := a[2] END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    #[test]
    fn array_index_must_be_integer() {
        let diags = check(
            "PROGRAM Foo; VAR a: ARRAY [1..10] OF INTEGER; b: BOOLEAN; \
             BEGIN a[TRUE] := 1 END.",
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("array index must be of type INTEGER"));
    }

    #[test]
    fn array_element_type_mismatch_is_reported() {
        let diags = check("PROGRAM Foo; VAR a: ARRAY [1..10] OF INTEGER; BEGIN a[1] := TRUE END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    #[test]
    fn indexing_a_non_array_type_is_an_error() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x[1] := 1 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("cannot index into non-array type"));
    }

    /// コンパイル時定数のリテラル添字に対する範囲チェック
    /// （タスク文書: 「範囲チェックは今回はコンパイル時定数に対してのみ行う」）。
    #[test]
    fn literal_out_of_bounds_array_index_is_reported() {
        let diags = check("PROGRAM Foo; VAR a: ARRAY [1..10] OF INTEGER; BEGIN a[20] := 1 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("out of bounds"));
    }

    /// 変数を添字に使う場合は、コンパイル時定数ではないため範囲チェックの
    /// 対象外（実行時範囲チェックは今回のスコープ外。タスク文書参照）。
    #[test]
    fn non_constant_array_index_is_not_range_checked() {
        let diags = check(
            "PROGRAM Foo; VAR a: ARRAY [1..10] OF INTEGER; i: INTEGER; \
             BEGIN i := 999; a[i] := 1 END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    #[test]
    fn multi_dimensional_array_element_access_is_well_typed() {
        let diags = check(
            "PROGRAM Foo; VAR a: ARRAY [1..10, 1..10] OF INTEGER; x: INTEGER; \
             BEGIN a[1, 2] := 5; x := a[3, 4] END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 同じ次元・要素型・添字範囲を持つ配列同士は構造的に同じ型とみなされ、
    /// 代入可能であること（タスク文書: 配列は構造的型付け）。
    #[test]
    fn structurally_identical_array_types_are_assignment_compatible() {
        let diags = check("PROGRAM Foo; VAR a, b: ARRAY [1..5] OF INTEGER; BEGIN a := b END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    #[test]
    fn array_types_with_different_bounds_are_not_assignment_compatible() {
        let diags = check(
            "PROGRAM Foo; VAR a: ARRAY [1..5] OF INTEGER; b: ARRAY [1..6] OF INTEGER; \
             BEGIN a := b END.",
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    // ---- レコード型 ----

    #[test]
    fn well_typed_record_declaration_and_field_access_has_no_diagnostics() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                Point = RECORD x, y: INTEGER END;
            VAR
                p: Point;
                n: INTEGER;
            BEGIN
                p.x := 1;
                p.y := 2;
                n := p.x + p.y
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    #[test]
    fn accessing_an_undeclared_field_is_an_error() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                Point = RECORD x, y: INTEGER END;
            VAR
                p: Point;
            BEGIN
                p.z := 1
            END.
        "#;
        let diags = check(src);
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("no field 'z'"));
    }

    #[test]
    fn accessing_a_field_of_a_non_record_type_is_an_error() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x.field := 1 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("cannot access field of non-record type"));
    }

    #[test]
    fn duplicate_field_name_in_a_record_is_an_error() {
        let diags = check("PROGRAM Foo; TYPE R = RECORD a: INTEGER; a: BOOLEAN END; BEGIN END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("already declared"));
    }

    /// 「同じ型宣言に由来するレコードのみ代入可能」という名前的型付けの
    /// ルール（タスク文書参照）: 同じ`TYPE`宣言由来のレコード同士は
    /// 代入可能。
    #[test]
    fn records_from_the_same_type_declaration_are_assignment_compatible() {
        let src = r#"
            PROGRAM Foo;
            TYPE Point = RECORD x, y: INTEGER END;
            VAR a, b: Point;
            BEGIN a := b END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 別々の`TYPE`宣言由来のレコードは、たとえフィールド構成が同一でも
    /// 代入できない（名前的型付け）。
    #[test]
    fn records_from_different_type_declarations_are_not_assignment_compatible() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                PointA = RECORD x, y: INTEGER END;
                PointB = RECORD x, y: INTEGER END;
            VAR a: PointA; b: PointB;
            BEGIN a := b END.
        "#;
        let diags = check(src);
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// `TYPE`宣言を経ない無名`RECORD`（`VAR`に直接書かれたもの）は、
    /// たとえ書かれた場所が異なっていても、それぞれ別の型として扱われる
    /// （`crate::typeck::SemaContext::resolve_record_type`のドキュメント参照）。
    #[test]
    fn anonymous_record_var_decls_are_each_their_own_type() {
        let diags = check(
            "PROGRAM Foo; VAR a: RECORD x: INTEGER END; b: RECORD x: INTEGER END; \
             BEGIN a := b END.",
        );
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    // ---- ポインタ型 ----

    #[test]
    fn well_typed_pointer_declaration_and_deref_has_no_diagnostics() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                Node = RECORD value: INTEGER END;
                PNode = ^Node;
            VAR
                p: PNode;
                n: INTEGER;
            BEGIN
                NEW(p);
                p^.value := 1;
                n := p^.value;
                DISPOSE(p)
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 前方参照を含む再帰的レコード定義（連結リスト的な構造）が解決できる
    /// こと（タスク文書の主要な検証項目）。
    #[test]
    fn forward_referenced_recursive_record_via_pointer_is_well_typed() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                PNode = ^Node;
                Node = RECORD
                    value: INTEGER;
                    next: PNode
                END;
            VAR
                head, cur: PNode;
            BEGIN
                head := NIL;
                NEW(head);
                head^.value := 1;
                head^.next := NIL;
                cur := head^.next;
                IF cur = NIL THEN
                    head^.value := 2
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 直接の自己参照（`PNode`のような別名を経由しない`^Node`）も解決できる。
    #[test]
    fn directly_self_referential_record_via_pointer_is_well_typed() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                Node = RECORD
                    value: INTEGER;
                    next: ^Node
                END;
            VAR
                head: ^Node;
            BEGIN
                head := NIL
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    #[test]
    fn dereferencing_a_non_pointer_type_is_an_error() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x^ := 1 END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0]
            .message
            .contains("cannot dereference non-pointer type"));
    }

    #[test]
    fn dereferencing_nil_directly_is_an_error() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN x := NIL^ END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("cannot dereference NIL"));
    }

    #[test]
    fn new_and_dispose_require_a_pointer_argument() {
        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN NEW(x) END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("expects a pointer-typed argument"));

        let diags = check("PROGRAM Foo; VAR x: INTEGER; BEGIN DISPOSE(x) END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("expects a pointer-typed argument"));
    }

    #[test]
    fn new_with_wrong_argument_count_is_an_error() {
        let diags = check("PROGRAM Foo; VAR p: ^INTEGER; BEGIN NEW(p, p) END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("expects 1 argument"));
    }

    #[test]
    fn nil_is_comparable_to_any_pointer_type() {
        let src = r#"
            PROGRAM Foo;
            VAR p: ^INTEGER; q: ^BOOLEAN; b: BOOLEAN;
            BEGIN
                b := p = NIL;
                b := NIL <> p;
                b := q = NIL
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    #[test]
    fn pointers_of_the_same_type_are_comparable() {
        let diags = check("PROGRAM Foo; VAR p, q: ^INTEGER; b: BOOLEAN; BEGIN b := p = q END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 異なる指す先の型を持つポインタ同士は比較できない
    /// （タスク文書: 「同じ指す先の型のポインタ同士のみ代入可能」という
    /// ルールを比較にも一貫して適用する）。
    #[test]
    fn pointers_of_different_pointee_types_are_not_comparable() {
        let diags =
            check("PROGRAM Foo; VAR p: ^INTEGER; q: ^BOOLEAN; b: BOOLEAN; BEGIN b := p = q END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    /// ポインタ型は`<`のような順序比較の対象にはならない
    /// （ISO 7185はポインタの大小比較を定義しない）。
    #[test]
    fn pointers_do_not_support_ordering_comparisons() {
        let diags = check("PROGRAM Foo; VAR p, q: ^INTEGER; b: BOOLEAN; BEGIN b := p < q END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    #[test]
    fn nil_is_assignable_to_any_pointer_type() {
        let diags = check("PROGRAM Foo; VAR p: ^INTEGER; BEGIN p := NIL END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    #[test]
    fn assigning_pointer_of_different_pointee_type_is_an_error() {
        let diags = check("PROGRAM Foo; VAR p: ^INTEGER; q: ^BOOLEAN; BEGIN p := q END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Type mismatch"));
    }

    #[test]
    fn passing_a_dereferenced_pointer_as_a_var_argument_is_allowed() {
        let src = r#"
            PROGRAM Foo;
            VAR p: ^INTEGER;

            PROCEDURE Bump(VAR n: INTEGER);
            BEGIN
                n := n + 1
            END;

            BEGIN
                NEW(p);
                p^ := 1;
                Bump(p^)
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    // ---- 未知の型名 ----

    #[test]
    fn referencing_an_undeclared_type_name_is_an_error() {
        let diags = check("PROGRAM Foo; VAR x: NoSuchType; BEGIN END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("Unknown type 'NoSuchType'"));
    }

    #[test]
    fn duplicate_type_declaration_is_an_error() {
        let diags = check("PROGRAM Foo; TYPE T = INTEGER; T = BOOLEAN; BEGIN END.");
        let errs = errors(&diags);
        assert_eq!(errs.len(), 1, "diagnostics: {diags:?}");
        assert!(errs[0].message.contains("already declared"));
    }

    // ---- 複合ケース ----

    /// レコードの配列。
    #[test]
    fn array_of_records_is_well_typed() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                Point = RECORD x, y: INTEGER END;
                Points = ARRAY [1..10] OF Point;
            VAR
                pts: Points;
                n: INTEGER;
            BEGIN
                pts[1].x := 1;
                pts[1].y := 2;
                n := pts[1].x + pts[1].y
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// 配列を含むレコード。
    #[test]
    fn record_containing_an_array_field_is_well_typed() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                Buffer = RECORD
                    data: ARRAY [1..10] OF INTEGER;
                    len: INTEGER
                END;
            VAR
                buf: Buffer;
                x: INTEGER;
            BEGIN
                buf.len := 0;
                buf.data[1] := 42;
                x := buf.data[1]
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// レコードへのポインタの配列（連結リストのノードへのポインタを
    /// 複数持つような構造を単純化した形）。
    #[test]
    fn array_of_pointers_to_records_is_well_typed() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                Node = RECORD value: INTEGER END;
                PNode = ^Node;
                PNodeArray = ARRAY [1..5] OF PNode;
            VAR
                nodes: PNodeArray;
                i: INTEGER;
            BEGIN
                FOR i := 1 TO 5 DO
                    nodes[i] := NIL;
                NEW(nodes[1]);
                nodes[1]^.value := 10
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    /// レコードのフィールドがポインタで、そのポインタが指す先がまた
    /// レコードの配列を含む、というやや深い組み合わせ。
    #[test]
    fn deeply_nested_record_array_pointer_combination_is_well_typed() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                Row = ARRAY [1..3] OF INTEGER;
                Grid = RECORD
                    rows: ARRAY [1..3] OF Row;
                    width, height: INTEGER
                END;
                PGrid = ^Grid;
            VAR
                g: PGrid;
                v: INTEGER;
            BEGIN
                NEW(g);
                g^.width := 3;
                g^.height := 3;
                g^.rows[1][2] := 7;
                v := g^.rows[1][2]
            END.
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    // ---- UNIT（前方参照はINTERFACE部のTYPEセクションでも解決できること）----

    #[test]
    fn unit_interface_type_section_supports_forward_referenced_records() {
        let src = r#"
            UNIT ListUnit;
            INTERFACE
            TYPE
                PNode = ^Node;
                Node = RECORD
                    value: INTEGER;
                    next: PNode
                END;
            VAR
                head: PNode;
            IMPLEMENTATION
            END.
        "#;
        let diags = check_unit_with_dialect(src, Dialect::Ucsd);
        let errs = errors(&diags);
        assert!(errs.is_empty(), "unexpected diagnostics: {diags:?}");
    }
}
