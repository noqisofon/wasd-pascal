//! 再帰下降パーサー本体。
//!
//! # スコープ
//!
//! `wasd-ast`の最小定義に一致させる:
//! - `PROGRAM <identifier>;` ヘッダ + 単一の`BEGIN...END.`ブロック
//! - `VAR`/`CONST`宣言（組み込み型`INTEGER`/`REAL`/`BOOLEAN`/`CHAR`のみ）
//! - `PROCEDURE`/`FUNCTION`宣言（仮引数リスト、`VAR`引数、ローカル`VAR`宣言）
//! - 文: 代入、`IF...THEN...[ELSE...]`、`WHILE...DO...`、複合文`BEGIN...END`、
//!   手続き呼び出し
//! - 式: リテラル、識別子、二項演算、単項演算、括弧、関数呼び出し
//!
//! `FOR`/`REPEAT`/`CASE`文に加え、UCSD拡張の`UNIT`/`INTERFACE`/
//! `IMPLEMENTATION`/`USES`、`CASE`文の`OTHERWISE`句、16進数リテラル
//! `$FF`、コンパイラディレクティブ、`STRING[n]`型もパースする。
//! 配列・レコード・ポインタ・集合型はまだパースしない（遭遇した場合は
//! 構文エラーの`Diagnostic`を発する）。
//!
//! # エントリポイント: `PROGRAM`か`UNIT`か
//!
//! [`Parser::parse_compilation_unit`]が先頭トークンを見て`PROGRAM`/`UNIT`の
//! どちらをパースするかを決める。[`Parser::parse_program`]/
//! [`Parser::parse_unit`]は個別に直接呼び出すこともできる（既存のテストや
//! 呼び出し元が`PROGRAM`前提で書かれている場合に備えて残す）。
//!
//! # エラー耐性（パニックモード回復）
//!
//! 構文エラーに遭遇してもパニック/中断せず、妥当な同期点（次のセミコロン、
//! 次の`END`/`ELSE`/`UNTIL`、あるいはEOF）まで読み飛ばして復帰し、可能な限り
//! 多くのエラーを1回のパースで報告する。式・型・文のいずれのパース関数も
//! （トップレベルの文パースを除き）失敗時にはプレースホルダのASTノードを
//! 返すことで、呼び出し元が`Option`の伝播に悩まされずに済むようにしている。
//! LSPでの利用（エラーを含むソースに対しても可能な限り完全な診断を返す）を
//! 想定した設計。

use wasd_ast::{
    BinOp, Block, CaseBranch, CompilationUnit, ConstDecl, Diagnostic, Expr, FieldDecl,
    ForDirection, FuncDecl, FuncSignature, Identifier, ImplementationSection, InterfaceSection,
    Literal, ParamDecl, ProcDecl, ProcSignature, Program, Severity, Span, Statement, TypeDecl,
    TypeExpr, Unit, UnOp, VarDecl,
};
use wasd_lexer::{Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    diagnostics: Vec<Diagnostic>,
}

impl Parser {
    pub fn new(mut tokens: Vec<Token>) -> Self {
        // レキサは常に末尾に`Eof`を付与するが、空の`Vec`を直接渡された場合に
        // `peek`/`advance`が範囲外アクセスにならないよう防御的に補う。
        if tokens.is_empty() {
            tokens.push(Token::new(TokenKind::Eof, Span::new(0, 0)));
        }
        Self {
            tokens,
            pos: 0,
            diagnostics: Vec::new(),
        }
    }

    /// ソース1本分のコンパイル単位（`PROGRAM`または`UNIT`）をパースする。
    /// 先頭トークンが`UNIT`であれば[`Self::parse_unit`]、それ以外は
    /// [`Self::parse_program`]に委譲する。
    pub fn parse_compilation_unit(&mut self) -> (Option<CompilationUnit>, Vec<Diagnostic>) {
        if matches!(self.peek(), TokenKind::Unit) {
            let (unit, diags) = self.parse_unit();
            (unit.map(CompilationUnit::Unit), diags)
        } else {
            let (program, diags) = self.parse_program();
            (program.map(CompilationUnit::Program), diags)
        }
    }

    /// プログラム全体をパースする。エラーがあってもパニックせず、可能な限り
    /// 復帰してパースを継続し、`Diagnostic`を蓄積する。
    ///
    /// 完全に何もパースできない場合（入力が空など）にのみ`None`を返す。
    /// それ以外は、エラーを含んでいてもベストエフォートで組み立てた
    /// `Program`を返す。
    pub fn parse_program(&mut self) -> (Option<Program>, Vec<Diagnostic>) {
        if self.is_eof() {
            let span = self.peek_span();
            self.error(span, "expected 'PROGRAM', found end of input");
            return (None, std::mem::take(&mut self.diagnostics));
        }

        let start = self.peek_span();
        self.expect(&TokenKind::Program, "'PROGRAM'");
        let name = self.parse_identifier("program name");
        self.expect(&TokenKind::Semicolon, "';'");

        let mut uses = Vec::new();
        let mut const_decls = Vec::new();
        let mut type_decls = Vec::new();
        let mut var_decls = Vec::new();
        let mut proc_decls = Vec::new();
        let mut func_decls = Vec::new();

        loop {
            match self.peek() {
                // UCSD拡張: `USES id, id, ...;`。標準的な置き場所は`PROGRAM`
                // ヘッダの直後だが、`CONST`/`VAR`同様、パーサーはどの順序でも
                // 受理する（dialectチェックは`wasd-sema`が行うため、パーサー
                // レベルでは位置に関する制約も設けない）。
                TokenKind::Uses => uses.extend(self.parse_uses_clause()),
                TokenKind::Const => const_decls.extend(self.parse_const_section()),
                TokenKind::Type => type_decls.extend(self.parse_type_section()),
                TokenKind::Var => var_decls.extend(self.parse_var_section()),
                TokenKind::Procedure => proc_decls.push(self.parse_proc_decl()),
                TokenKind::Function => func_decls.push(self.parse_func_decl()),
                TokenKind::Label | TokenKind::Unit => {
                    let kind = self.peek().clone();
                    let span = self.peek_span();
                    self.error(
                        span,
                        format!(
                            "{} declarations are not supported by this parser yet",
                            describe(&kind)
                        ),
                    );
                    self.skip_unsupported_section();
                }
                _ => break,
            }
        }

        let body = self.parse_subprogram_body();

        self.expect(&TokenKind::Dot, "'.'");

        let end = self.previous_span().end.max(start.end);
        let program = Program {
            name,
            uses,
            const_decls,
            type_decls,
            var_decls,
            proc_decls,
            func_decls,
            body,
            span: Span::new(start.start, end),
        };

        (Some(program), std::mem::take(&mut self.diagnostics))
    }

    // ------------------------------------------------------------------
    // UCSD拡張: UNIT / INTERFACE / IMPLEMENTATION / USES
    // ------------------------------------------------------------------

    /// `UNIT name; INTERFACE ... IMPLEMENTATION ... END.`
    ///
    /// UNCONFIRMED: `IMPLEMENTATION`部の末尾（`END.`の直前）に初期化用の
    /// 文の並びが書けるかどうかは、Pascal言語レベルの正確な構文としては
    /// 一次資料で未確認のため、今回は実装しない（p-machine内部仕様レベルでは
    /// `'***'`セグメント参照を通じた初期化・終了処理の存在自体は確認できて
    /// いる。`wasd_ast::Unit`のドキュメント参照）。
    fn parse_unit(&mut self) -> (Option<Unit>, Vec<Diagnostic>) {
        if self.is_eof() {
            let span = self.peek_span();
            self.error(span, "expected 'UNIT', found end of input");
            return (None, std::mem::take(&mut self.diagnostics));
        }

        let start = self.peek_span();
        self.expect(&TokenKind::Unit, "'UNIT'");
        let name = self.parse_identifier("unit name");
        self.expect(&TokenKind::Semicolon, "';'");

        let interface = self.parse_interface_section();
        let implementation = self.parse_implementation_section();

        let end_span = self.expect_and_span(&TokenKind::End, "'END'");
        self.expect(&TokenKind::Dot, "'.'");

        let end = end_span.end.max(start.end);
        let unit = Unit {
            name,
            interface,
            implementation,
            span: Span::new(start.start, end),
        };

        (Some(unit), std::mem::take(&mut self.diagnostics))
    }

    /// `INTERFACE [USES ...;] [CONST ...] [VAR ...] {PROCEDURE/FUNCTION シグネチャ}`
    fn parse_interface_section(&mut self) -> InterfaceSection {
        let start = self.peek_span();
        self.expect(&TokenKind::Interface, "'INTERFACE'");

        let mut uses = Vec::new();
        let mut const_decls = Vec::new();
        let mut type_decls = Vec::new();
        let mut var_decls = Vec::new();
        let mut proc_signatures = Vec::new();
        let mut func_signatures = Vec::new();

        loop {
            match self.peek() {
                TokenKind::Uses => uses.extend(self.parse_uses_clause()),
                TokenKind::Const => const_decls.extend(self.parse_const_section()),
                TokenKind::Type => type_decls.extend(self.parse_type_section()),
                TokenKind::Var => var_decls.extend(self.parse_var_section()),
                TokenKind::Procedure => proc_signatures.push(self.parse_proc_signature()),
                TokenKind::Function => func_signatures.push(self.parse_func_signature()),
                _ => break,
            }
        }

        let end = self.previous_span().end.max(start.end);
        InterfaceSection {
            uses,
            const_decls,
            type_decls,
            var_decls,
            proc_signatures,
            func_signatures,
            span: Span::new(start.start, end),
        }
    }

    /// `IMPLEMENTATION {PROCEDURE/FUNCTION 完全な宣言}`
    fn parse_implementation_section(&mut self) -> ImplementationSection {
        let start = self.peek_span();
        self.expect(&TokenKind::Implementation, "'IMPLEMENTATION'");

        let mut proc_decls = Vec::new();
        let mut func_decls = Vec::new();
        loop {
            match self.peek() {
                TokenKind::Procedure => proc_decls.push(self.parse_proc_decl()),
                TokenKind::Function => func_decls.push(self.parse_func_decl()),
                _ => break,
            }
        }

        let end = self.previous_span().end.max(start.end);
        ImplementationSection {
            proc_decls,
            func_decls,
            span: Span::new(start.start, end),
        }
    }

    /// `USES id, id, ...;`。呼び出し元は`self.peek()`が`TokenKind::Uses`で
    /// あることを確認済みの前提。
    fn parse_uses_clause(&mut self) -> Vec<Identifier> {
        self.advance(); // USES
        let mut names = vec![self.parse_identifier("unit name")];
        while self.check(&TokenKind::Comma) {
            self.advance();
            names.push(self.parse_identifier("unit name"));
        }
        self.expect(&TokenKind::Semicolon, "';'");
        names
    }

    /// `INTERFACE`部の`PROCEDURE name(params);`（本体なし）。
    fn parse_proc_signature(&mut self) -> ProcSignature {
        let start = self.peek_span();
        self.advance(); // PROCEDURE
        let name = self.parse_identifier("procedure name");
        let params = self.parse_param_list();
        self.expect(&TokenKind::Semicolon, "';'");
        let end = self.previous_span().end;
        ProcSignature {
            name,
            params,
            span: Span::new(start.start, end),
        }
    }

    /// `INTERFACE`部の`FUNCTION name(params): returnType;`（本体なし）。
    fn parse_func_signature(&mut self) -> FuncSignature {
        let start = self.peek_span();
        self.advance(); // FUNCTION
        let name = self.parse_identifier("function name");
        let params = self.parse_param_list();
        self.expect(&TokenKind::Colon, "':'");
        let return_type = self.parse_type();
        self.expect(&TokenKind::Semicolon, "';'");
        let end = self.previous_span().end;
        FuncSignature {
            name,
            params,
            return_type,
            span: Span::new(start.start, end),
        }
    }

    // ------------------------------------------------------------------
    // 宣言
    // ------------------------------------------------------------------

    fn parse_const_section(&mut self) -> Vec<ConstDecl> {
        self.advance(); // CONST
        let mut decls = Vec::new();
        while matches!(self.peek(), TokenKind::Identifier(_)) {
            let name = self.parse_identifier("constant name");
            self.expect(&TokenKind::Eq, "'='");
            let value = self.parse_const_literal();
            self.expect(&TokenKind::Semicolon, "';'");
            let span = Span::new(name.span.start, value.span().end);
            decls.push(ConstDecl { name, value, span });
        }
        decls
    }

    /// `CONST`宣言の右辺。任意の符号 + 数値/文字列/真偽値リテラルのみを許す
    /// （`wasd_ast::decl`のドキュメント通り、定数式や他の定数への参照は
    /// 今回のスコープ外）。
    fn parse_const_literal(&mut self) -> Literal {
        let start = self.peek_span();
        let negate = match self.peek() {
            TokenKind::Minus => {
                self.advance();
                true
            }
            TokenKind::Plus => {
                self.advance();
                false
            }
            _ => false,
        };

        match self.peek().clone() {
            TokenKind::IntegerLiteral(v) => {
                self.advance();
                let v = if negate { -v } else { v };
                Literal::Int(v, Span::new(start.start, self.previous_span().end))
            }
            // UCSD拡張: 16進数リテラル `$FF`。
            //
            // # 既知のスコープ制限: `CONST`宣言・`CASE`ラベルではdialectチェックを
            //   行わない
            //
            // `wasd_ast::Literal`（`CONST`宣言の右辺・`CASE`ラベルで使う）は
            // `Expr`と異なり16進数由来かどうかを区別するバリアントを持たない
            // （式中の`Expr::HexIntLiteral`とは非対称）。式中の`$FF`
            // （`x := $FF`のような代入・比較の右辺）についてはdialectチェックを
            // 実装するが、`CONST`宣言や`CASE`ラベルの中の`$FF`については
            // 値だけを普通の`Literal::Int`としてデコードし、dialectチェックは
            // 行わない。これは今回のスコープを絞るための意図的な制限であり、
            // 将来`Literal`にも同様の区別を持たせる形で解消できる
            // （TODO: `Literal::HexInt`のようなバリアントを追加する）。
            TokenKind::HexIntegerLiteral(v) => {
                self.advance();
                let v = if negate { -v } else { v };
                Literal::Int(v, Span::new(start.start, self.previous_span().end))
            }
            TokenKind::RealLiteral(v) => {
                self.advance();
                let v = if negate { -v } else { v };
                Literal::Real(v, Span::new(start.start, self.previous_span().end))
            }
            TokenKind::StringLiteral(s) => {
                self.advance();
                if negate {
                    self.error(start, "unary '-' cannot be applied to a string literal");
                }
                Literal::Str(s, Span::new(start.start, self.previous_span().end))
            }
            TokenKind::Identifier(name) => {
                self.advance();
                let lower = name.to_ascii_lowercase();
                if lower == "true" || lower == "false" {
                    if negate {
                        self.error(start, "unary '-' cannot be applied to a boolean literal");
                    }
                    Literal::Bool(lower == "true", Span::new(start.start, self.previous_span().end))
                } else {
                    self.error(
                        start,
                        format!(
                            "expected a literal value for CONST declaration, found identifier '{name}' (references to other constants are not supported yet)"
                        ),
                    );
                    Literal::Int(0, start)
                }
            }
            other => {
                self.error(start, format!("expected a literal value, found {}", describe(&other)));
                if !matches!(other, TokenKind::Semicolon | TokenKind::Eof) {
                    self.advance();
                }
                Literal::Int(0, start)
            }
        }
    }

    fn parse_var_section(&mut self) -> Vec<VarDecl> {
        self.advance(); // VAR
        let mut decls = Vec::new();
        while matches!(self.peek(), TokenKind::Identifier(_)) {
            let start = self.peek_span();
            let mut names = vec![self.parse_identifier("variable name")];
            while self.check(&TokenKind::Comma) {
                self.advance();
                names.push(self.parse_identifier("variable name"));
            }
            self.expect(&TokenKind::Colon, "':'");
            let ty = self.parse_type();
            self.expect(&TokenKind::Semicolon, "';'");
            let span = Span::new(start.start, ty.span().end);
            decls.push(VarDecl { names, ty, span });
        }
        decls
    }

    /// `TYPE name1 = TypeExpr; name2 = TypeExpr; ...`
    ///
    /// 宣言順序は`wasd_ast::decl::Program::type_decls`のドキュメント通り
    /// 意味を持つ（`wasd-sema`によるポインタの前方参照解決のため）ため、
    /// ソース上の並び順をそのまま保持する。
    fn parse_type_section(&mut self) -> Vec<TypeDecl> {
        self.advance(); // TYPE
        let mut decls = Vec::new();
        while matches!(self.peek(), TokenKind::Identifier(_)) {
            let name = self.parse_identifier("type name");
            self.expect(&TokenKind::Eq, "'='");
            let ty = self.parse_type();
            self.expect(&TokenKind::Semicolon, "';'");
            let span = Span::new(name.span.start, ty.span().end);
            decls.push(TypeDecl { name, ty, span });
        }
        decls
    }

    /// 型注釈のパース。組み込み型・`STRING[n]`・配列・レコード・ポインタ型に
    /// 加え、まだ組み込み型として認識できない識別子は`TYPE`セクションで
    /// 宣言された（あるいはこれから宣言される）型名への参照
    /// (`TypeExpr::Named`) として受理する。実際にその名前が存在するかどうかの
    /// 検査は構文解析の範囲外であり、`wasd-sema`が行う
    /// （`wasd_ast::decl::TypeExpr::Named`のドキュメント参照）。
    fn parse_type(&mut self) -> TypeExpr {
        let packed = self.check(&TokenKind::Packed);
        let packed_span = self.peek_span();
        if packed {
            self.advance();
        }

        let span = self.peek_span();
        let ty = match self.peek().clone() {
            TokenKind::Array => self.parse_array_type(if packed { packed_span } else { span }, packed),
            TokenKind::Record => self.parse_record_type(if packed { packed_span } else { span }, packed),
            TokenKind::Caret => self.parse_pointer_type(span),
            TokenKind::Identifier(name) => {
                self.advance();
                match name.to_ascii_lowercase().as_str() {
                    "integer" => TypeExpr::Integer(span),
                    "real" => TypeExpr::Real(span),
                    "boolean" => TypeExpr::Boolean(span),
                    "char" => TypeExpr::Char(span),
                    "string" => self.parse_string_n_type(span),
                    _ => TypeExpr::Named(Identifier::new(name, span)),
                }
            }
            other => {
                self.error(
                    span,
                    format!(
                        "expected a type (INTEGER/REAL/BOOLEAN/CHAR/STRING[n]/ARRAY/RECORD/^type/a type name), found {}",
                        describe(&other)
                    ),
                );
                if !matches!(other, TokenKind::Semicolon | TokenKind::Eof) {
                    self.advance();
                }
                TypeExpr::Integer(span)
            }
        };

        if packed && !matches!(ty, TypeExpr::Array { .. } | TypeExpr::Record { .. }) {
            self.error(packed_span, "'PACKED' can only be applied to ARRAY or RECORD types");
        }
        ty
    }

    /// `ARRAY [index_type_list] OF element_type`。呼び出し元は`PACKED`の
    /// 有無を判定済みで、`self.peek()`が`TokenKind::Array`であることが前提。
    ///
    /// # 設計判断: 多次元配列はネストした`Array`に展開する
    ///
    /// `ARRAY [1..10, 1..20] OF INTEGER`は`ARRAY [1..10] OF ARRAY [1..20] OF
    /// INTEGER`の糖衣構文として扱う（`wasd_ast::decl::TypeExpr`の
    /// ドキュメント参照）。`index_type_list`を右から畳み込み、最も内側の
    /// 次元が`element_type`に最も近い`Array`になるようにする。
    fn parse_array_type(&mut self, start: Span, packed: bool) -> TypeExpr {
        self.advance(); // ARRAY
        self.expect(&TokenKind::LBracket, "'['");

        let mut index_types = vec![self.parse_subrange_index()];
        while self.check(&TokenKind::Comma) {
            self.advance();
            index_types.push(self.parse_subrange_index());
        }
        self.expect(&TokenKind::RBracket, "']'");
        self.expect(&TokenKind::Of, "'OF'");
        let element_type = self.parse_type();
        let end = element_type.span().end.max(start.end);

        let mut result = element_type;
        for index_type in index_types.into_iter().rev() {
            result = TypeExpr::Array {
                index_type: Box::new(index_type),
                element_type: Box::new(result),
                packed,
                span: Span::new(start.start, end),
            };
        }
        result
    }

    /// 配列添字の1次元分、`low..high`（今回は`INTEGER`のリテラルのみ対応。
    /// `wasd_ast::decl::TypeExpr::Subrange`のドキュメント参照）。
    fn parse_subrange_index(&mut self) -> TypeExpr {
        let low = self.parse_const_literal();
        self.expect(&TokenKind::DotDot, "'..'");
        let high = self.parse_const_literal();
        let span = Span::new(low.span().start, high.span().end);
        TypeExpr::Subrange { low, high, span }
    }

    /// `RECORD field1, field2: T1; field3: T2; ... END`。呼び出し元は
    /// `PACKED`の有無を判定済みで、`self.peek()`が`TokenKind::Record`で
    /// あることが前提。
    fn parse_record_type(&mut self, start: Span, packed: bool) -> TypeExpr {
        self.advance(); // RECORD
        let fields = self.parse_field_list();
        let end_span = self.expect_and_span(&TokenKind::End, "'END'");
        TypeExpr::Record {
            fields,
            packed,
            span: Span::new(start.start, end_span.end.max(start.end)),
        }
    }

    /// `RECORD`本体のフィールド並び。`parse_var_section`と同じ形
    /// （`names: T;`の繰り返し）だが、終端が`END`である点が異なる
    /// （呼び出し元が`END`を消費する）。
    fn parse_field_list(&mut self) -> Vec<FieldDecl> {
        let mut fields = Vec::new();
        while matches!(self.peek(), TokenKind::Identifier(_)) {
            let start = self.peek_span();
            let mut names = vec![self.parse_identifier("field name")];
            while self.check(&TokenKind::Comma) {
                self.advance();
                names.push(self.parse_identifier("field name"));
            }
            self.expect(&TokenKind::Colon, "':'");
            let ty = self.parse_type();
            let span = Span::new(start.start, ty.span().end);
            fields.push(FieldDecl { names, ty, span });

            if self.check(&TokenKind::Semicolon) {
                self.advance();
            } else {
                break;
            }
        }
        fields
    }

    /// `^T`（ポインタ型）。呼び出し元は`self.peek()`が`TokenKind::Caret`で
    /// あることが前提。
    ///
    /// `T`は`Node`のようにまだ`TYPE`宣言されていない可能性がある
    /// （前方参照）。パーサー自身は構文的に受理するだけで、名前解決
    /// （同じ`TYPE`セクション内でのレコード型への前方参照を含む）は
    /// `wasd-sema`が行う。
    fn parse_pointer_type(&mut self, caret_span: Span) -> TypeExpr {
        self.advance(); // ^
        let inner = self.parse_type();
        let span = Span::new(caret_span.start, inner.span().end.max(caret_span.end));
        TypeExpr::Pointer(Box::new(inner), span)
    }

    /// UCSD拡張: `STRING[n]`型。呼び出し元は`STRING`識別子を消費済みで、
    /// `type_start`はその`STRING`識別子自体のspan（`[n]`を含まない）。
    ///
    /// UNCONFIRMED: 角括弧`[n]`を省略した`STRING`単体の宣言が許されるか、
    /// 許される場合の既定最大長は何かは一次資料で未確認のまま（2026-09-01の
    /// 一次資料調査でも未調査。継続調査が必要。リポジトリのUCSD Pascal
    /// 一次資料調査メモ参照）。ここでは慣用的によく引用される既定値`80`を
    /// 仮に採用し、パーサーレベルでは拒否しない（`wasd_ast::Dialect`の設計方針どおり、
    /// UCSD拡張構文は常に受理する）。
    fn parse_string_n_type(&mut self, type_start: Span) -> TypeExpr {
        if !self.check(&TokenKind::LBracket) {
            return TypeExpr::StringN(80, type_start);
        }
        self.advance(); // '['

        let len = match self.peek().clone() {
            TokenKind::IntegerLiteral(v) if v > 0 => {
                self.advance();
                v as usize
            }
            other => {
                self.error(
                    self.peek_span(),
                    format!(
                        "expected a positive integer length for STRING[n], found {}",
                        describe(&other)
                    ),
                );
                if !matches!(other, TokenKind::RBracket | TokenKind::Semicolon | TokenKind::Eof) {
                    self.advance();
                }
                80
            }
        };

        let close = self.expect_and_span(&TokenKind::RBracket, "']'");
        TypeExpr::StringN(len, Span::new(type_start.start, close.end.max(type_start.end)))
    }

    /// `PROCEDURE name(params); [VAR ...] BEGIN ... END;`
    fn parse_proc_decl(&mut self) -> ProcDecl {
        let start = self.peek_span();
        self.advance(); // PROCEDURE
        let name = self.parse_identifier("procedure name");
        let params = self.parse_param_list();
        self.expect(&TokenKind::Semicolon, "';'");
        let var_decls = self.parse_local_declarations();
        let body = self.parse_subprogram_body();
        self.expect(&TokenKind::Semicolon, "';'");
        let end = self.previous_span().end;
        ProcDecl {
            name,
            params,
            var_decls,
            body,
            span: Span::new(start.start, end),
        }
    }

    /// `FUNCTION name(params): returnType; [VAR ...] BEGIN ... END;`
    fn parse_func_decl(&mut self) -> FuncDecl {
        let start = self.peek_span();
        self.advance(); // FUNCTION
        let name = self.parse_identifier("function name");
        let params = self.parse_param_list();
        self.expect(&TokenKind::Colon, "':'");
        let return_type = self.parse_type();
        self.expect(&TokenKind::Semicolon, "';'");
        let var_decls = self.parse_local_declarations();
        let body = self.parse_subprogram_body();
        self.expect(&TokenKind::Semicolon, "';'");
        let end = self.previous_span().end;
        FuncDecl {
            name,
            params,
            return_type,
            var_decls,
            body,
            span: Span::new(start.start, end),
        }
    }

    fn parse_subprogram_body(&mut self) -> Block {
        match self.peek() {
            TokenKind::Begin => match self.parse_compound_statement() {
                Some(Statement::Compound(block)) => block,
                _ => Block {
                    statements: vec![],
                    span: self.peek_span(),
                },
            },
            other => {
                let span = self.peek_span();
                let found = describe(other);
                self.error(span, format!("expected 'BEGIN', found {found}"));
                Block {
                    statements: vec![],
                    span,
                }
            }
        }
    }

    /// 仮引数リスト`(a, b: integer; VAR c: real)`。括弧自体が省略された
    /// （引数なしの）場合は空の`Vec`を返す。
    fn parse_param_list(&mut self) -> Vec<ParamDecl> {
        let mut params = Vec::new();
        if !self.check(&TokenKind::LParen) {
            return params;
        }
        self.advance(); // (
        if self.check(&TokenKind::RParen) {
            self.advance();
            return params;
        }

        loop {
            let by_ref = self.eat(&TokenKind::Var);
            let mut names = vec![self.parse_identifier("parameter name")];
            while self.check(&TokenKind::Comma) {
                self.advance();
                names.push(self.parse_identifier("parameter name"));
            }
            self.expect(&TokenKind::Colon, "':'");
            let ty = self.parse_type();
            for name in names {
                let span = Span::new(name.span.start, ty.span().end);
                params.push(ParamDecl {
                    name,
                    ty: ty.clone(),
                    by_ref,
                    span,
                });
            }

            if self.check(&TokenKind::Semicolon) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(&TokenKind::RParen, "')'");
        params
    }

    /// `PROCEDURE`/`FUNCTION`本体内のローカル宣言。今回のスコープでは
    /// ローカル`VAR`宣言のみをサポートする（ローカル`CONST`宣言、
    /// ネストした`PROCEDURE`/`FUNCTION`は今回のスコープ外）。
    fn parse_local_declarations(&mut self) -> Vec<VarDecl> {
        let mut var_decls = Vec::new();
        loop {
            match self.peek() {
                TokenKind::Var => var_decls.extend(self.parse_var_section()),
                TokenKind::Const
                | TokenKind::Type
                | TokenKind::Procedure
                | TokenKind::Function
                | TokenKind::Label => {
                    let kind = self.peek().clone();
                    let span = self.peek_span();
                    self.error(
                        span,
                        format!(
                            "{} declarations are not supported inside a PROCEDURE/FUNCTION body yet (only VAR sections are supported)",
                            describe(&kind)
                        ),
                    );
                    // このアームに来るトークン(CONST/VAR以外)は`skip_unsupported_
                    // section`が停止条件として扱わないため、まずここで1つ消費して
                    // から読み飛ばしを行う。`CONST`はスキップの停止条件に含まれる
                    // ため、先に消費しておかないと同じ`CONST`に対して無限に
                    // このアームへ戻ってきてしまう。
                    self.advance();
                    self.skip_unsupported_section();
                }
                _ => break,
            }
        }
        var_decls
    }

    /// `PROCEDURE`/`FUNCTION`/`TYPE`/`UNIT`など、まだ対応していない宣言
    /// セクションを読み飛ばす。`BEGIN`/`END`のネストを大まかに数え、
    /// トップレベルの`CONST`/`VAR`/`BEGIN`（本体の開始）まで読み飛ばす。
    fn skip_unsupported_section(&mut self) {
        let mut depth: i32 = 0;
        loop {
            match self.peek() {
                TokenKind::Eof => return,
                TokenKind::Begin => {
                    if depth == 0 {
                        return;
                    }
                    depth += 1;
                    self.advance();
                }
                TokenKind::End => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    self.advance();
                }
                TokenKind::Const | TokenKind::Var if depth == 0 => return,
                _ => {
                    self.advance();
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // 文
    // ------------------------------------------------------------------

    fn parse_statement(&mut self) -> Option<Statement> {
        let kind = self.peek().clone();
        match kind {
            TokenKind::Begin => self.parse_compound_statement(),
            TokenKind::If => self.parse_if_statement(),
            TokenKind::While => self.parse_while_statement(),
            TokenKind::For => self.parse_for_statement(),
            TokenKind::Repeat => self.parse_repeat_statement(),
            TokenKind::Case => self.parse_case_statement(),
            TokenKind::Identifier(_) => self.parse_assignment_or_call(),

            // UCSD拡張: コンパイラディレクティブ `(*$I foo.pas*)`。
            // `wasd_ast::Statement::CompilerDirective`のドキュメント参照。
            TokenKind::CompilerDirective { name, args } => {
                let span = self.peek_span();
                self.advance();
                Some(Statement::CompilerDirective { name, args, span })
            }

            // 空文（empty statement）。ISO 7185の文法は`statement`の一種として
            // 「何もない」ことを許容する（`;;`の連続や`THEN`直後の`ELSE`など）。
            // ここでは呼び出し元がこれらのトークンを消費しない前提で、
            // 長さ0のCompound文として扱う。
            TokenKind::Semicolon | TokenKind::End | TokenKind::Else | TokenKind::Until => {
                let span = self.peek_span();
                Some(Statement::Compound(Block {
                    statements: vec![],
                    span: Span::new(span.start, span.start),
                }))
            }

            TokenKind::With | TokenKind::Goto | TokenKind::Label => {
                let span = self.peek_span();
                self.error(
                    span,
                    format!(
                        "{} is not supported by this parser yet (only assignment, IF/THEN/ELSE, WHILE/DO, FOR/DO, REPEAT/UNTIL, CASE/OF, compound statements, and procedure calls are supported)",
                        describe(&kind)
                    ),
                );
                self.advance();
                None
            }

            other => {
                let span = self.peek_span();
                self.error(span, format!("expected statement, found {}", describe(&other)));
                None
            }
        }
    }

    /// `parse_statement`のエラー耐性版。パースに失敗した場合は次の同期点まで
    /// 読み飛ばし、空のCompound文をプレースホルダとして返す。呼び出し元
    /// （`IF`/`WHILE`の本体、複合文中の各文）が`Option`を扱わずに済むようにする。
    fn parse_statement_or_recover(&mut self) -> Statement {
        if let Some(stmt) = self.parse_statement() {
            stmt
        } else {
            let span = self.peek_span();
            self.synchronize_to_statement_boundary();
            Statement::Compound(Block {
                statements: vec![],
                span: Span::new(span.start, span.start),
            })
        }
    }

    fn parse_compound_statement(&mut self) -> Option<Statement> {
        let start = self.peek_span();
        if !self.eat(&TokenKind::Begin) {
            let span = self.peek_span();
            let found = describe(self.peek());
            self.error(span, format!("expected 'BEGIN', found {found}"));
            return None;
        }

        let statements = self.parse_statement_sequence(&TokenKind::End, "'END'");
        let end_span = self.expect_and_span(&TokenKind::End, "'END'");
        Some(Statement::Compound(Block {
            statements,
            span: Span::new(start.start, end_span.end),
        }))
    }

    /// `;`区切りの文の並びを、`terminator`（`BEGIN...END`なら`END`、
    /// `REPEAT...UNTIL`なら`UNTIL`）の手前までパースする。`terminator`
    /// 自体は消費せず、呼び出し元が消費する。
    fn parse_statement_sequence(&mut self, terminator: &TokenKind, terminator_desc: &str) -> Vec<Statement> {
        let mut statements = Vec::new();
        loop {
            // 連続するセミコロン（空文）を読み飛ばす。
            while self.check(&TokenKind::Semicolon) {
                self.advance();
            }
            if self.check(terminator) || self.is_eof() {
                break;
            }

            match self.parse_statement() {
                Some(stmt) => {
                    statements.push(stmt);

                    if self.check(&TokenKind::Semicolon) {
                        continue;
                    } else if self.check(terminator) || self.is_eof() {
                        break;
                    } else {
                        let span = self.peek_span();
                        let found = describe(self.peek());
                        self.error(
                            span,
                            format!("expected ';' or {terminator_desc}, found {found}"),
                        );
                        // `synchronize_to_statement_boundary`は`ELSE`/`UNTIL`の
                        // 手前で止まる。ここに来る`ELSE`/`UNTIL`は（`IF`/`REPEAT`
                        // に対応しない）迷子のトークンであり、`parse_statement`は
                        // これらを消費しない空文として返すため、何もせず
                        // 素通りすると同じ位置に戻ってきて無限ループになる。
                        // 同期処理が全く前進しなかった場合は、無限ループ防止の
                        // ため強制的に1トークン読み飛ばす。
                        let pos_before_sync = self.pos;
                        self.synchronize_to_statement_boundary();
                        if self.pos == pos_before_sync {
                            self.advance();
                        }
                    }
                }
                None => {
                    // `parse_statement`が構文エラーで諦めた場合、既に`Diagnostic`は
                    // 積まれている。同期点（次の`;`、あるいは`END`/`ELSE`/`UNTIL`の
                    // 手前）まで読み飛ばして次の文から継続する。`synchronize_to_
                    // statement_boundary`が区切りのセミコロンを消費し得るため、
                    // ここではプレースホルダの文をASTに追加しない
                    // （追加すると、直後の正常な文を「区切りのセミコロンがない」
                    // と誤って再度エラーにしてしまう）。
                    self.synchronize_to_statement_boundary();
                }
            }
        }
        statements
    }

    /// `REPEAT stmt1; stmt2; ... UNTIL cond`
    fn parse_repeat_statement(&mut self) -> Option<Statement> {
        let start = self.peek_span();
        self.advance(); // REPEAT
        let body = self.parse_statement_sequence(&TokenKind::Until, "'UNTIL'");
        self.expect(&TokenKind::Until, "'UNTIL'");
        let until_cond = self.parse_expr();
        let span = Span::new(start.start, until_cond.span().end);
        Some(Statement::Repeat {
            body,
            until_cond,
            span,
        })
    }

    /// `CASE selector OF label1, label2: stmt1; label3: stmt2; [OTHERWISE stmtN] END`
    ///
    /// UCSD拡張の`OTHERWISE`句（どのラベルにも一致しない場合のデフォルト
    /// 分岐）もパースする。文法上、`OTHERWISE`は最後の分岐として現れる想定
    /// （その後にさらに`label: stmt`形式の分岐は続かない）ため、`OTHERWISE`
    /// を読んだ時点でループを抜ける。dialectチェック（`Iso7185`では使用不可）
    /// は`wasd-sema`が行う。
    fn parse_case_statement(&mut self) -> Option<Statement> {
        let start = self.peek_span();
        self.advance(); // CASE
        let selector = self.parse_expr();
        self.expect(&TokenKind::Of, "'OF'");

        let mut branches = Vec::new();
        let mut otherwise = None;
        loop {
            while self.check(&TokenKind::Semicolon) {
                self.advance();
            }
            if self.check(&TokenKind::End) || self.is_eof() {
                break;
            }

            if self.check(&TokenKind::Otherwise) {
                self.advance();
                let stmt = self.parse_statement_or_recover();
                otherwise = Some(Box::new(stmt));
                if self.check(&TokenKind::Semicolon) {
                    self.advance();
                }
                break;
            }

            let branch_start = self.peek_span();
            let mut labels = vec![self.parse_const_literal()];
            while self.check(&TokenKind::Comma) {
                self.advance();
                labels.push(self.parse_const_literal());
            }
            self.expect(&TokenKind::Colon, "':'");
            let body = self.parse_statement_or_recover();
            let branch_span = Span::new(branch_start.start, body.span().end);
            branches.push(CaseBranch {
                labels,
                body,
                span: branch_span,
            });

            if self.check(&TokenKind::Semicolon) {
                continue;
            } else if self.check(&TokenKind::End) || self.is_eof() {
                break;
            } else if self.check(&TokenKind::Otherwise) {
                // `OTHERWISE`の前のセミコロンは省略できる
                // （`1: stmt OTHERWISE stmt2`）。ループの先頭に戻れば
                // `OTHERWISE`ハンドリングに入る。
                continue;
            } else {
                let span = self.peek_span();
                let found = describe(self.peek());
                self.error(span, format!("expected ';' or 'END', found {found}"));
                let pos_before_sync = self.pos;
                self.synchronize_to_statement_boundary();
                if self.pos == pos_before_sync {
                    self.advance();
                }
            }
        }

        let end_span = self.expect_and_span(&TokenKind::End, "'END'");
        Some(Statement::Case {
            selector,
            branches,
            otherwise,
            span: Span::new(start.start, end_span.end),
        })
    }

    fn parse_if_statement(&mut self) -> Option<Statement> {
        let start = self.peek_span();
        self.advance(); // IF
        let cond = self.parse_expr();
        self.expect(&TokenKind::Then, "'THEN'");
        let then_branch = self.parse_statement_or_recover();

        // dangling-else: `ELSE`は直近の未対応`IF`に対応させる（greedy match）。
        // ここで即座に`ELSE`の有無を確認することで自然にこの挙動になる。
        let (else_branch, end) = if self.check(&TokenKind::Else) {
            self.advance();
            let else_stmt = self.parse_statement_or_recover();
            let end = else_stmt.span().end;
            (Some(Box::new(else_stmt)), end)
        } else {
            (None, then_branch.span().end)
        };

        let span = Span::new(start.start, end);
        Some(Statement::If {
            cond,
            then_branch: Box::new(then_branch),
            else_branch,
            span,
        })
    }

    fn parse_while_statement(&mut self) -> Option<Statement> {
        let start = self.peek_span();
        self.advance(); // WHILE
        let cond = self.parse_expr();
        self.expect(&TokenKind::Do, "'DO'");
        let body = self.parse_statement_or_recover();
        let span = Span::new(start.start, body.span().end);
        Some(Statement::While {
            cond,
            body: Box::new(body),
            span,
        })
    }

    /// `FOR var := start (TO|DOWNTO) end DO body`
    fn parse_for_statement(&mut self) -> Option<Statement> {
        let start = self.peek_span();
        self.advance(); // FOR
        let var = self.parse_identifier("loop variable");
        self.expect(&TokenKind::Assign, "':='");
        let start_expr = self.parse_expr();

        let direction = if self.eat(&TokenKind::To) {
            ForDirection::To
        } else if self.eat(&TokenKind::DownTo) {
            ForDirection::DownTo
        } else {
            let span = self.peek_span();
            let found = describe(self.peek());
            self.error(span, format!("expected 'TO' or 'DOWNTO', found {found}"));
            ForDirection::To
        };

        let end_expr = self.parse_expr();
        self.expect(&TokenKind::Do, "'DO'");
        let body = self.parse_statement_or_recover();
        let span = Span::new(start.start, body.span().end);
        Some(Statement::For {
            var,
            start: start_expr,
            end: end_expr,
            direction,
            body: Box::new(body),
            span,
        })
    }

    /// 代入文 `designator := expr` と手続き呼び出し文
    /// `identifier(expr, ...)` / `identifier` は共に識別子から始まるため、
    /// ここでまとめて先読み分岐する。
    ///
    /// 代入文の左辺（`designator`）は単純な識別子とは限らず、配列添字
    /// アクセス・レコードフィールドアクセス・ポインタデリファレンスの
    /// 組み合わせ（`arr[i].field^`など）にもなり得る。手続き呼び出しの
    /// 括弧`(args)`は識別子に直接続く場合のみを認識する（`arr[i](x)`の
    /// ような構文はこの言語のサブセットには存在しない）ため、まず
    /// `(`の有無を確認し、なければ識別子を起点に後置演算子チェーン
    /// （[`Self::parse_postfix_chain`]）を読み取ってから`:=`の有無を見る。
    fn parse_assignment_or_call(&mut self) -> Option<Statement> {
        let name = self.parse_identifier("identifier");

        if self.check(&TokenKind::LParen) {
            self.advance();
            let args = self.parse_call_args();
            let close = self.expect_and_span(&TokenKind::RParen, "')'");
            let span = Span::new(name.span.start, close.end.max(name.span.end));
            return Some(Statement::ProcCall { name, args, span });
        }

        let designator = self.parse_postfix_chain(Expr::Identifier(name.clone()));

        if self.check(&TokenKind::Assign) {
            self.advance();
            let value = self.parse_expr();
            let span = Span::new(designator.span().start, value.span().end);
            Some(Statement::Assignment { target: designator, value, span })
        } else if matches!(designator, Expr::Identifier(_)) {
            let span = name.span;
            Some(Statement::ProcCall { name, args: vec![], span })
        } else {
            let span = self.peek_span();
            let found = describe(self.peek());
            self.error(
                span,
                format!("expected ':=' after array/record/pointer designator, found {found}"),
            );
            None
        }
    }

    // ------------------------------------------------------------------
    // 式（演算子優先順位）
    //
    // # 根拠: ISO/IEC 7185:1990, 6.7.1 "Expressions"
    //
    // 規格の構文規則（簡略化して引用）:
    //
    // ```text
    // expression           = simple-expression [ relational-operator simple-expression ] .
    // simple-expression    = [ sign ] term { adding-operator term } .
    // term                 = factor { multiplying-operator factor } .
    // factor               = variable-access | unsigned-constant | function-designator
    //                      | set-constructor | "(" expression ")" | "not" factor .
    // adding-operator      = "+" | "-" | "or" .
    // multiplying-operator = "*" | "/" | "div" | "mod" | "and" .
    // relational-operator  = "=" | "<>" | "<" | ">" | "<=" | ">=" | "in" .
    // ```
    //
    // この文法から導かれる優先順位（緩い→強い、構文木の根に近い順）:
    //
    // 1. 関係演算子 (`= <> < > <= >=`) — `expression`の最外周。左右に高々
    //    1回だけ出現でき、連鎖できない（`a < b < c`は文法上不正）。
    // 2. 加算レベル: `+ - OR`（`simple-expression`）
    // 3. 乗算レベル: `* / DIV MOD AND`（`term`）— **`AND`は`OR`と同じ階層では
    //    なく乗算演算子群に属する**点に注意（規格の`multiplying-operator`の
    //    定義を参照。`AND`は`term`レベル、`OR`は`simple-expression`レベル）。
    // 4. 単項: `NOT`（`factor`内で再帰的に定義）、単項`-`/`+`（`simple-expression`
    //    の`sign`）。規格上`sign`は先頭に一度だけ許されるが、本実装は簡潔さの
    //    ため`NOT`と同様に再帰的な単項演算子として扱う。通常書かれる式の範囲
    //    では観測可能な差異はない（`--x`のような構文は規格上も定義が曖昧な
    //    境界事例であり、今回のスコープでは問題にならない）。
    // 5. `factor`: リテラル・識別子・括弧
    //
    // したがって `a AND b OR c` は `(a AND b) OR c` と解釈される
    // （`AND`が`term`レベル、`OR`が`simple-expression`レベルのため、
    // `AND`の方が強く結合する）。
    // ------------------------------------------------------------------

    fn parse_expr(&mut self) -> Expr {
        let lhs = self.parse_simple_expr();

        let Some(op) = self.peek_relop() else {
            return lhs;
        };
        self.advance();
        let rhs = self.parse_simple_expr();
        let span = Span::new(lhs.span().start, rhs.span().end);
        let expr = Expr::BinaryOp {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        };

        if self.peek_relop().is_some() {
            let extra_span = self.peek_span();
            self.error(
                extra_span,
                "relational operators cannot be chained (e.g. 'a < b < c'); use parentheses to group comparisons",
            );
        }

        expr
    }

    fn peek_relop(&self) -> Option<BinOp> {
        match self.peek() {
            TokenKind::Eq => Some(BinOp::Eq),
            TokenKind::Ne => Some(BinOp::NotEq),
            TokenKind::Lt => Some(BinOp::Lt),
            TokenKind::Gt => Some(BinOp::Gt),
            TokenKind::Le => Some(BinOp::LtEq),
            TokenKind::Ge => Some(BinOp::GtEq),
            _ => None,
        }
    }

    fn parse_simple_expr(&mut self) -> Expr {
        let mut lhs = self.parse_term();
        loop {
            let op = match self.peek() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                TokenKind::Or => BinOp::Or,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_term();
            let span = Span::new(lhs.span().start, rhs.span().end);
            lhs = Expr::BinaryOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        lhs
    }

    fn parse_term(&mut self) -> Expr {
        let mut lhs = self.parse_unary();
        loop {
            let op = match self.peek() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Div => BinOp::IntDiv,
                TokenKind::Mod => BinOp::Mod,
                TokenKind::And => BinOp::And,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_unary();
            let span = Span::new(lhs.span().start, rhs.span().end);
            lhs = Expr::BinaryOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        lhs
    }

    fn parse_unary(&mut self) -> Expr {
        match self.peek() {
            TokenKind::Not => {
                let start = self.peek_span();
                self.advance();
                let operand = self.parse_unary();
                let span = Span::new(start.start, operand.span().end);
                Expr::UnaryOp {
                    op: UnOp::Not,
                    operand: Box::new(operand),
                    span,
                }
            }
            TokenKind::Minus => {
                let start = self.peek_span();
                self.advance();
                let operand = self.parse_unary();
                let span = Span::new(start.start, operand.span().end);
                Expr::UnaryOp {
                    op: UnOp::Neg,
                    operand: Box::new(operand),
                    span,
                }
            }
            TokenKind::Plus => {
                // 単項`+`は意味を持たないので、消費するだけでノードは作らない。
                self.advance();
                self.parse_unary()
            }
            _ => self.parse_postfix(),
        }
    }

    /// `parse_primary`が返した式に対し、後置演算子（配列添字`[..]`、
    /// レコードフィールドアクセス`.field`、ポインタデリファレンス`^`）の
    /// 並びを可能な限り貪欲に読み取る。`x[i].field^[j]`のような組み合わせも
    /// 許可する。
    fn parse_postfix(&mut self) -> Expr {
        let expr = self.parse_primary();
        self.parse_postfix_chain(expr)
    }

    /// [`Self::parse_postfix`]の本体。代入文の左辺（`parse_assignment_or_call`）
    /// でも、既に読み終えた識別子を起点に同じ後置演算子チェーンを組み立てる
    /// ために共有する。
    fn parse_postfix_chain(&mut self, mut expr: Expr) -> Expr {
        loop {
            match self.peek() {
                TokenKind::LBracket => {
                    self.advance();
                    // `arr[i, j]`は`arr[i][j]`と同じ意味の糖衣構文として、
                    // 左結合的にネストした`IndexAccess`に展開する
                    // （`wasd_ast::expr::Expr::IndexAccess`のドキュメント参照）。
                    loop {
                        let index = self.parse_expr();
                        let span = Span::new(expr.span().start, index.span().end);
                        expr = Expr::IndexAccess {
                            array: Box::new(expr),
                            index: Box::new(index),
                            span,
                        };
                        if self.check(&TokenKind::Comma) {
                            self.advance();
                            continue;
                        }
                        break;
                    }
                    let close = self.expect_and_span(&TokenKind::RBracket, "']'");
                    if let Expr::IndexAccess { span, .. } = &mut expr {
                        *span = Span::new(span.start, close.end.max(span.end));
                    }
                }
                TokenKind::Dot => {
                    self.advance();
                    let field = self.parse_identifier("field name");
                    let span = Span::new(expr.span().start, field.span.end);
                    expr = Expr::FieldAccess {
                        record: Box::new(expr),
                        field,
                        span,
                    };
                }
                TokenKind::Caret => {
                    let caret_span = self.peek_span();
                    self.advance();
                    let span = Span::new(expr.span().start, caret_span.end);
                    expr = Expr::Deref {
                        pointer: Box::new(expr),
                        span,
                    };
                }
                _ => break,
            }
        }
        expr
    }

    fn parse_primary(&mut self) -> Expr {
        let span = self.peek_span();
        match self.peek().clone() {
            TokenKind::IntegerLiteral(v) => {
                self.advance();
                Expr::IntLiteral(v, span)
            }
            TokenKind::Nil => {
                self.advance();
                Expr::NilLiteral(span)
            }
            // UCSD拡張: 16進数リテラル `$FF`。`Expr::IntLiteral`とは別の
            // `Expr::HexIntLiteral`として組み立てる（dialectチェックを
            // `wasd-sema`で行うため。`wasd_ast::Expr::HexIntLiteral`の
            // ドキュメント参照）。パーサー自身はdialectに関わらず常に受理する。
            TokenKind::HexIntegerLiteral(v) => {
                self.advance();
                Expr::HexIntLiteral(v, span)
            }
            TokenKind::RealLiteral(v) => {
                self.advance();
                Expr::RealLiteral(v, span)
            }
            TokenKind::StringLiteral(s) => {
                self.advance();
                Expr::StringLiteral(s, span)
            }
            TokenKind::Identifier(name) => {
                self.advance();
                let lower = name.to_ascii_lowercase();
                if lower == "true" {
                    Expr::BoolLiteral(true, span)
                } else if lower == "false" {
                    Expr::BoolLiteral(false, span)
                } else if self.check(&TokenKind::LParen) {
                    // `identifier(args)`: 括弧を伴う場合は関数呼び出し式として
                    // 組み立てる。括弧を省略した引数なしの関数呼び出し
                    // （`x := Foo`）は、この時点では単なる変数参照と構文上
                    // 区別が付かないため、`Expr::Identifier`のままにして
                    // 意味解析側で解決する（`wasd_ast::expr::Expr::FuncCall`
                    // のドキュメント参照）。
                    self.advance();
                    let args = self.parse_call_args();
                    let close = self.expect_and_span(&TokenKind::RParen, "')'");
                    Expr::FuncCall {
                        name: Identifier::new(name, span),
                        args,
                        span: Span::new(span.start, close.end.max(span.end)),
                    }
                } else {
                    Expr::Identifier(Identifier::new(name, span))
                }
            }
            TokenKind::LParen => {
                self.advance();
                let inner = self.parse_expr();
                let close = self.expect_and_span(&TokenKind::RParen, "')'");
                Expr::Paren(Box::new(inner), Span::new(span.start, close.end))
            }
            other => {
                self.error(span, format!("expected expression, found {}", describe(&other)));
                // 明らかに式の一部になり得ない同期用トークンは消費せずに
                // 呼び出し元へ戻す（無限ループ防止と、上位の同期処理に委ねるため）。
                if !matches!(
                    other,
                    TokenKind::Semicolon
                        | TokenKind::End
                        | TokenKind::Then
                        | TokenKind::Do
                        | TokenKind::Else
                        | TokenKind::RParen
                        | TokenKind::Comma
                        | TokenKind::Eof
                ) {
                    self.advance();
                }
                Expr::IntLiteral(0, span)
            }
        }
    }

    // ------------------------------------------------------------------
    // 共通ヘルパー
    // ------------------------------------------------------------------

    /// 呼び出し式の実引数リスト。呼び出し元が既に開き括弧`(`を消費済みで、
    /// 閉じ括弧`)`はまだ消費していない前提（閉じ括弧の消費は呼び出し元が行う）。
    fn parse_call_args(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                args.push(self.parse_expr());
                if self.check(&TokenKind::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        args
    }

    fn parse_identifier(&mut self, what: &str) -> Identifier {
        match self.peek().clone() {
            TokenKind::Identifier(name) => {
                let span = self.peek_span();
                self.advance();
                Identifier::new(name, span)
            }
            other => {
                let span = self.peek_span();
                self.error(span, format!("expected {what}, found {}", describe(&other)));
                Identifier::new(String::new(), span)
            }
        }
    }

    /// 構文エラーからの回復（パニックモード）。次のセミコロンまで読み飛ばし
    /// （セミコロン自体は消費する）、あるいは`END`/`ELSE`/`UNTIL`/EOFの
    /// 直前で止まる（これらは消費せず、呼び出し元の判断に委ねる）。
    fn synchronize_to_statement_boundary(&mut self) {
        loop {
            match self.peek() {
                TokenKind::Semicolon => {
                    self.advance();
                    return;
                }
                TokenKind::End | TokenKind::Else | TokenKind::Until | TokenKind::Eof => return,
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn previous_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    fn is_eof(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    /// バリアントの種類だけを比較する（データを持つ`Identifier`/`IntegerLiteral`
    /// などは中身を無視して種類だけ一致すればよいため）。
    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(kind)
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> bool {
        if self.eat(kind) {
            true
        } else {
            let span = self.peek_span();
            let found = describe(self.peek());
            self.error(span, format!("expected {what}, found {found}"));
            false
        }
    }

    fn expect_and_span(&mut self, kind: &TokenKind, what: &str) -> Span {
        if self.expect(kind, what) {
            self.previous_span()
        } else {
            self.peek_span()
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(span, Severity::Error, message));
    }
}

/// 診断メッセージ用に、トークン種別を人間が読める形の文字列にする。
/// 「何を期待していたか」/「実際に何を見つけたか」の両方でこの関数を使う
/// ことで、メッセージの語彙を統一する。
fn describe(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Program => "'PROGRAM'".to_string(),
        TokenKind::Begin => "'BEGIN'".to_string(),
        TokenKind::End => "'END'".to_string(),
        TokenKind::Var => "'VAR'".to_string(),
        TokenKind::Const => "'CONST'".to_string(),
        TokenKind::Type => "'TYPE'".to_string(),
        TokenKind::Procedure => "'PROCEDURE'".to_string(),
        TokenKind::Function => "'FUNCTION'".to_string(),
        TokenKind::If => "'IF'".to_string(),
        TokenKind::Then => "'THEN'".to_string(),
        TokenKind::Else => "'ELSE'".to_string(),
        TokenKind::While => "'WHILE'".to_string(),
        TokenKind::Do => "'DO'".to_string(),
        TokenKind::Repeat => "'REPEAT'".to_string(),
        TokenKind::Until => "'UNTIL'".to_string(),
        TokenKind::For => "'FOR'".to_string(),
        TokenKind::To => "'TO'".to_string(),
        TokenKind::DownTo => "'DOWNTO'".to_string(),
        TokenKind::Case => "'CASE'".to_string(),
        TokenKind::Of => "'OF'".to_string(),
        TokenKind::Record => "'RECORD'".to_string(),
        TokenKind::Array => "'ARRAY'".to_string(),
        TokenKind::Set => "'SET'".to_string(),
        TokenKind::File => "'FILE'".to_string(),
        TokenKind::Packed => "'PACKED'".to_string(),
        TokenKind::Label => "'LABEL'".to_string(),
        TokenKind::Goto => "'GOTO'".to_string(),
        TokenKind::With => "'WITH'".to_string(),
        TokenKind::Nil => "'NIL'".to_string(),
        TokenKind::Not => "'NOT'".to_string(),
        TokenKind::And => "'AND'".to_string(),
        TokenKind::Or => "'OR'".to_string(),
        TokenKind::Div => "'DIV'".to_string(),
        TokenKind::Mod => "'MOD'".to_string(),
        TokenKind::In => "'IN'".to_string(),
        TokenKind::Unit => "'UNIT'".to_string(),
        TokenKind::Interface => "'INTERFACE'".to_string(),
        TokenKind::Implementation => "'IMPLEMENTATION'".to_string(),
        TokenKind::Uses => "'USES'".to_string(),
        TokenKind::Otherwise => "'OTHERWISE'".to_string(),
        TokenKind::Identifier(name) => format!("identifier '{name}'"),
        TokenKind::IntegerLiteral(v) => format!("integer literal '{v}'"),
        TokenKind::HexIntegerLiteral(v) => format!("hexadecimal literal '${v:X}'"),
        TokenKind::RealLiteral(v) => format!("real literal '{v}'"),
        TokenKind::StringLiteral(s) => format!("string literal '{s}'"),
        TokenKind::Assign => "':='".to_string(),
        TokenKind::Le => "'<='".to_string(),
        TokenKind::Ge => "'>='".to_string(),
        TokenKind::Ne => "'<>'".to_string(),
        TokenKind::DotDot => "'..'".to_string(),
        TokenKind::Plus => "'+'".to_string(),
        TokenKind::Minus => "'-'".to_string(),
        TokenKind::Star => "'*'".to_string(),
        TokenKind::Slash => "'/'".to_string(),
        TokenKind::Eq => "'='".to_string(),
        TokenKind::Lt => "'<'".to_string(),
        TokenKind::Gt => "'>'".to_string(),
        TokenKind::LParen => "'('".to_string(),
        TokenKind::RParen => "')'".to_string(),
        TokenKind::LBracket => "'['".to_string(),
        TokenKind::RBracket => "']'".to_string(),
        TokenKind::Dot => "'.'".to_string(),
        TokenKind::Comma => "','".to_string(),
        TokenKind::Semicolon => "';'".to_string(),
        TokenKind::Colon => "':'".to_string(),
        TokenKind::Caret => "'^'".to_string(),
        TokenKind::CompilerDirective { name, .. } => format!("compiler directive '${name}'"),
        TokenKind::Eof => "end of input".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_source(source: &str) -> (Option<Program>, Vec<Diagnostic>) {
        let (tokens, lex_diags) = wasd_lexer::Lexer::new(source).tokenize();
        assert!(
            lex_diags.is_empty(),
            "unexpected lexer diagnostics for {source:?}: {lex_diags:?}"
        );
        Parser::new(tokens).parse_program()
    }

    /// テスト対象(1): 最小プログラムが正しくパースされる。
    #[test]
    fn parses_minimal_program() {
        let (program, diags) = parse_source("PROGRAM Foo; BEGIN END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");
        assert_eq!(program.name.name, "Foo");
        assert!(program.const_decls.is_empty());
        assert!(program.var_decls.is_empty());
        assert!(program.body.statements.is_empty());
    }

    /// テスト対象(2): VAR/CONST宣言を含むプログラムが正しくパースされる。
    #[test]
    fn parses_var_and_const_sections() {
        let src = r#"
            PROGRAM Foo;
            CONST
                MaxScore = 100;
                Ratio = 3.25;
            VAR
                x, y: INTEGER;
                flag: BOOLEAN;
            BEGIN
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        assert_eq!(program.const_decls.len(), 2);
        assert_eq!(program.const_decls[0].name.name, "MaxScore");
        match &program.const_decls[0].value {
            Literal::Int(v, _) => assert_eq!(*v, 100),
            other => panic!("expected Int literal, got {other:?}"),
        }
        match &program.const_decls[1].value {
            Literal::Real(v, _) => assert_eq!(*v, 3.25),
            other => panic!("expected Real literal, got {other:?}"),
        }

        assert_eq!(program.var_decls.len(), 2);
        let names: Vec<&str> = program.var_decls[0]
            .names
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(names, vec!["x", "y"]);
        assert!(matches!(program.var_decls[0].ty, TypeExpr::Integer(_)));
        assert!(matches!(program.var_decls[1].ty, TypeExpr::Boolean(_)));
    }

    /// テスト対象(3a): dangling-elseは最も近い未対応のIFに対応する。
    #[test]
    fn dangling_else_binds_to_nearest_if() {
        let src = r#"
            PROGRAM Foo;
            VAR x: INTEGER;
            BEGIN
                IF x THEN
                    IF x THEN
                        x := 1
                    ELSE
                        x := 2
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                assert!(else_branch.is_none(), "outer IF should have no ELSE");
                match then_branch.as_ref() {
                    Statement::If {
                        else_branch: inner_else,
                        ..
                    } => {
                        assert!(inner_else.is_some(), "ELSE should bind to the inner IF");
                    }
                    other => panic!("expected nested IF, got {other:?}"),
                }
            }
            other => panic!("expected IF statement, got {other:?}"),
        }
    }

    /// テスト対象(3b): ネストしたWHILEが正しい構造でパースされる。
    #[test]
    fn parses_nested_while_inside_if() {
        let src = r#"
            PROGRAM Foo;
            VAR x: INTEGER;
            BEGIN
                IF x THEN
                    WHILE x DO
                        x := x - 1
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::If { then_branch, .. } => {
                assert!(matches!(then_branch.as_ref(), Statement::While { .. }));
            }
            other => panic!("expected IF statement, got {other:?}"),
        }
    }

    /// テスト対象(4a): 加減算より乗除算が強く結合する（`1 + 2 * 3`）。
    #[test]
    fn respects_additive_and_multiplicative_precedence() {
        let (program, diags) = parse_source("PROGRAM Foo; VAR x: INTEGER; BEGIN x := 1 + 2 * 3 END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Assignment { value, .. } => match value {
                Expr::BinaryOp {
                    op: BinOp::Add,
                    lhs,
                    rhs,
                    ..
                } => {
                    assert!(matches!(lhs.as_ref(), Expr::IntLiteral(1, _)));
                    match rhs.as_ref() {
                        Expr::BinaryOp {
                            op: BinOp::Mul,
                            lhs,
                            rhs,
                            ..
                        } => {
                            assert!(matches!(lhs.as_ref(), Expr::IntLiteral(2, _)));
                            assert!(matches!(rhs.as_ref(), Expr::IntLiteral(3, _)));
                        }
                        other => panic!("expected multiplication on the rhs, got {other:?}"),
                    }
                }
                other => panic!("expected addition at the top, got {other:?}"),
            },
            other => panic!("expected an assignment statement, got {other:?}"),
        }
    }

    /// テスト対象(4b): `a AND b OR c`は`(a AND b) OR c`（AND優先）と解釈される。
    #[test]
    fn and_binds_tighter_than_or() {
        let (program, diags) =
            parse_source("PROGRAM Foo; VAR a, b, c: BOOLEAN; BEGIN a := a AND b OR c END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Assignment { value, .. } => match value {
                Expr::BinaryOp {
                    op: BinOp::Or,
                    lhs,
                    rhs,
                    ..
                } => {
                    assert!(matches!(rhs.as_ref(), Expr::Identifier(id) if id.name == "c"));
                    assert!(matches!(
                        lhs.as_ref(),
                        Expr::BinaryOp {
                            op: BinOp::And,
                            ..
                        }
                    ));
                }
                other => panic!("expected OR at the top, got {other:?}"),
            },
            other => panic!("expected an assignment statement, got {other:?}"),
        }
    }

    /// テスト対象(5): 構文エラーを含む入力でもパニックせず`Diagnostic`が返り、
    /// エラーの後続の妥当な文が正しくパースされる。
    #[test]
    fn recovers_from_syntax_error_and_continues_parsing() {
        let src = r#"
            PROGRAM Foo;
            VAR x: INTEGER;
            BEGIN
                x := ;
                x := 42
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(!diags.is_empty(), "expected at least one diagnostic");
        let program = program.expect("parser should still produce a Program despite the error");

        assert_eq!(program.body.statements.len(), 2);
        match &program.body.statements[1] {
            Statement::Assignment { value, .. } => {
                assert!(matches!(value, Expr::IntLiteral(42, _)));
            }
            other => panic!("expected the recovered assignment, got {other:?}"),
        }
    }

    /// 未対応構文（FOR）に遭遇してもパニックせず、Diagnosticを出しつつ
    /// 後続の文のパースを継続できる。
    #[test]
    fn reports_diagnostic_for_unsupported_goto_statement_without_panicking() {
        let src = r#"
            PROGRAM Foo;
            VAR x: INTEGER;
            BEGIN
                GOTO 1;
                x := 1
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(!diags.is_empty(), "expected at least one diagnostic for GOTO");
        let program = program.expect("parser should still produce a Program despite the error");
        assert_eq!(program.body.statements.len(), 1);
        match &program.body.statements[0] {
            Statement::Assignment { value, .. } => {
                assert!(matches!(value, Expr::IntLiteral(1, _)));
            }
            other => panic!("expected the recovered assignment, got {other:?}"),
        }
    }

    /// 対応する`REPEAT`のない迷子の`UNTIL`に遭遇してもパーサーが無限ループに
    /// 陥らないこと。`UNTIL`は`parse_statement`の「空文」扱いのトークン集合
    /// （`;`/`END`/`ELSE`/`UNTIL`）に含まれるため、同期処理が前進せず
    /// 無限ループになる回帰が過去にあった。
    #[test]
    fn does_not_hang_on_stray_until_token() {
        let src = r#"
            PROGRAM Foo;
            VAR x: INTEGER;
            BEGIN
                UNTIL x = 1;
                x := 1
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(!diags.is_empty(), "expected at least one diagnostic for the stray UNTIL");
        assert!(
            program.is_some(),
            "parser should still produce a Program despite the error"
        );
    }

    /// `FOR ... TO ... DO`が正しくパースされること。
    #[test]
    fn parses_for_to_statement() {
        let src = r#"
            PROGRAM Foo;
            VAR i, x: INTEGER;
            BEGIN
                FOR i := 1 TO 10 DO
                    x := i
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::For {
                var,
                start,
                end,
                direction,
                body,
                ..
            } => {
                assert_eq!(var.name, "i");
                assert!(matches!(start, Expr::IntLiteral(1, _)));
                assert!(matches!(end, Expr::IntLiteral(10, _)));
                assert_eq!(*direction, ForDirection::To);
                assert!(matches!(body.as_ref(), Statement::Assignment { .. }));
            }
            other => panic!("expected a For statement, got {other:?}"),
        }
    }

    /// `FOR ... DOWNTO ... DO`が正しくパースされること。
    #[test]
    fn parses_for_downto_statement() {
        let (program, diags) = parse_source(
            "PROGRAM Foo; VAR i: INTEGER; BEGIN FOR i := 10 DOWNTO 1 DO i := i END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::For { direction, .. } => {
                assert_eq!(*direction, ForDirection::DownTo);
            }
            other => panic!("expected a For statement, got {other:?}"),
        }
    }

    /// `FOR`の本体にBEGIN...ENDの複合文を使えること（複数文のループ本体）。
    #[test]
    fn parses_for_with_compound_body() {
        let src = r#"
            PROGRAM Foo;
            VAR i, sum: INTEGER;
            BEGIN
                sum := 0;
                FOR i := 1 TO 10 DO
                BEGIN
                    sum := sum + i
                END
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[1] {
            Statement::For { body, .. } => {
                assert!(matches!(body.as_ref(), Statement::Compound(_)));
            }
            other => panic!("expected a For statement, got {other:?}"),
        }
    }

    /// `REPEAT ... UNTIL`が`BEGIN...END`なしで複数文を直接パースできること。
    #[test]
    fn parses_repeat_until_with_multiple_statements() {
        let src = r#"
            PROGRAM Foo;
            VAR i: INTEGER;
            BEGIN
                i := 0;
                REPEAT
                    i := i + 1;
                    i := i + 1
                UNTIL i >= 10
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[1] {
            Statement::Repeat { body, until_cond, .. } => {
                assert_eq!(body.len(), 2);
                assert!(matches!(
                    until_cond,
                    Expr::BinaryOp {
                        op: BinOp::GtEq,
                        ..
                    }
                ));
            }
            other => panic!("expected a Repeat statement, got {other:?}"),
        }
    }

    /// `REPEAT`の本体が1文だけの場合もパースできること。
    #[test]
    fn parses_repeat_until_with_single_statement() {
        let (program, diags) =
            parse_source("PROGRAM Foo; VAR i: INTEGER; BEGIN i := 0; REPEAT i := i + 1 UNTIL i = 1 END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[1] {
            Statement::Repeat { body, .. } => {
                assert_eq!(body.len(), 1);
            }
            other => panic!("expected a Repeat statement, got {other:?}"),
        }
    }

    /// `CASE`文が複数の分岐（カンマ区切りの複数ラベルを含む）で正しく
    /// パースされること。
    #[test]
    fn parses_case_statement_with_multiple_branches() {
        let src = r#"
            PROGRAM Foo;
            VAR x, y: INTEGER;
            BEGIN
                CASE x OF
                    1, 2: y := 1;
                    3: y := 2
                END
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Case { selector, branches, .. } => {
                assert!(matches!(selector, Expr::Identifier(id) if id.name == "x"));
                assert_eq!(branches.len(), 2);
                assert_eq!(branches[0].labels.len(), 2);
                match &branches[0].labels[0] {
                    Literal::Int(v, _) => assert_eq!(*v, 1),
                    other => panic!("expected an Int literal, got {other:?}"),
                }
                match &branches[0].labels[1] {
                    Literal::Int(v, _) => assert_eq!(*v, 2),
                    other => panic!("expected an Int literal, got {other:?}"),
                }
                assert_eq!(branches[1].labels.len(), 1);
                assert!(matches!(branches[0].body, Statement::Assignment { .. }));
            }
            other => panic!("expected a Case statement, got {other:?}"),
        }
    }

    /// 末尾のセミコロン（最後の分岐の後の`;`）を許容すること。
    #[test]
    fn parses_case_statement_with_trailing_semicolon() {
        let (program, diags) = parse_source(
            "PROGRAM Foo; VAR x: INTEGER; BEGIN CASE x OF 1: x := 1; END END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Case { branches, .. } => {
                assert_eq!(branches.len(), 1);
            }
            other => panic!("expected a Case statement, got {other:?}"),
        }
    }

    /// テスト対象(6): wasd-lexerでトークン化 -> wasd-parserでパース、という
    /// 一連の流れの統合テスト。手続き呼び出し（引数あり/なし）を含む。
    #[test]
    fn integration_lexes_and_parses_procedure_calls() {
        let src = r#"
            PROGRAM Greet;
            VAR name: INTEGER;
            BEGIN
                name := 1;
                WriteLn(name);
                WriteLn
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        assert_eq!(program.body.statements.len(), 3);
        match &program.body.statements[1] {
            Statement::ProcCall { name, args, .. } => {
                assert_eq!(name.name, "WriteLn");
                assert_eq!(args.len(), 1);
            }
            other => panic!("expected a ProcCall, got {other:?}"),
        }
        match &program.body.statements[2] {
            Statement::ProcCall { name, args, .. } => {
                assert_eq!(name.name, "WriteLn");
                assert!(args.is_empty());
            }
            other => panic!("expected an argument-less ProcCall, got {other:?}"),
        }
    }

    /// `PROCEDURE`宣言（`VAR`引数を含む）が正しくパースされること。
    #[test]
    fn parses_procedure_decl_with_var_param() {
        let src = r#"
            PROGRAM Foo;
            PROCEDURE Swap(VAR a, b: INTEGER; c: BOOLEAN);
            VAR tmp: INTEGER;
            BEGIN
                tmp := a;
                a := b;
                b := tmp
            END;
            BEGIN
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        assert_eq!(program.proc_decls.len(), 1);
        let proc = &program.proc_decls[0];
        assert_eq!(proc.name.name, "Swap");
        assert_eq!(proc.params.len(), 3);
        assert_eq!(proc.params[0].name.name, "a");
        assert!(proc.params[0].by_ref);
        assert!(matches!(proc.params[0].ty, TypeExpr::Integer(_)));
        assert_eq!(proc.params[1].name.name, "b");
        assert!(proc.params[1].by_ref);
        assert_eq!(proc.params[2].name.name, "c");
        assert!(!proc.params[2].by_ref);
        assert!(matches!(proc.params[2].ty, TypeExpr::Boolean(_)));
        assert_eq!(proc.var_decls.len(), 1);
        assert_eq!(proc.body.statements.len(), 3);
    }

    /// `FUNCTION`宣言（戻り値の型と、伝統的な`FunctionName := value`による
    /// 戻り値設定）が正しくパースされること。戻り値設定はAST上では
    /// 通常の`Statement::Assignment`としてパースされる（意味解析側で
    /// 「関数名への代入」として解釈する方針のため）。
    #[test]
    fn parses_function_decl_with_return_type_and_self_assignment() {
        let src = r#"
            PROGRAM Foo;
            FUNCTION Square(x: INTEGER): INTEGER;
            BEGIN
                Square := x * x
            END;
            BEGIN
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        assert_eq!(program.func_decls.len(), 1);
        let func = &program.func_decls[0];
        assert_eq!(func.name.name, "Square");
        assert_eq!(func.params.len(), 1);
        assert_eq!(func.params[0].name.name, "x");
        assert!(matches!(func.return_type, TypeExpr::Integer(_)));
        assert_eq!(func.body.statements.len(), 1);
        match &func.body.statements[0] {
            Statement::Assignment { target, value, .. } => {
                assert!(matches!(target, Expr::Identifier(id) if id.name == "Square"));
                assert!(matches!(
                    value,
                    Expr::BinaryOp {
                        op: BinOp::Mul,
                        ..
                    }
                ));
            }
            other => panic!("expected an Assignment, got {other:?}"),
        }
    }

    /// 関数呼び出し式`Foo(1, 2)`が`Expr::FuncCall`としてパースされること。
    #[test]
    fn parses_function_call_expression() {
        let (program, diags) = parse_source(
            "PROGRAM Foo; VAR x: INTEGER; BEGIN x := Add(1, 2) END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Assignment { value, .. } => match value {
                Expr::FuncCall { name, args, .. } => {
                    assert_eq!(name.name, "Add");
                    assert_eq!(args.len(), 2);
                }
                other => panic!("expected a FuncCall, got {other:?}"),
            },
            other => panic!("expected an Assignment, got {other:?}"),
        }
    }

    /// 引数なしの関数呼び出し（括弧省略）は`Expr::Identifier`としてパースされ、
    /// 意味解析側での解決に委ねられること。
    #[test]
    fn niladic_function_call_without_parens_parses_as_identifier() {
        let (program, diags) =
            parse_source("PROGRAM Foo; VAR x: INTEGER; BEGIN x := GetAnswer END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Assignment { value, .. } => {
                assert!(matches!(value, Expr::Identifier(id) if id.name == "GetAnswer"));
            }
            other => panic!("expected an Assignment, got {other:?}"),
        }
    }

    /// 再帰呼び出し（関数本体内での自分自身の呼び出し）が構文上パースできること。
    #[test]
    fn parses_recursive_function_call_in_body() {
        let src = r#"
            PROGRAM Foo;
            FUNCTION Fact(n: INTEGER): INTEGER;
            BEGIN
                IF n <= 1 THEN
                    Fact := 1
                ELSE
                    Fact := n * Fact(n - 1)
            END;
            BEGIN
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");
        assert_eq!(program.func_decls.len(), 1);
    }

    // ------------------------------------------------------------------
    // Step 7: UCSD拡張構文（パーサーはdialectに関わらず常に受理する）
    // ------------------------------------------------------------------

    fn parse_unit_source(source: &str) -> (Option<wasd_ast::CompilationUnit>, Vec<Diagnostic>) {
        let (tokens, lex_diags) = wasd_lexer::Lexer::new(source).tokenize();
        assert!(
            lex_diags.is_empty(),
            "unexpected lexer diagnostics for {source:?}: {lex_diags:?}"
        );
        Parser::new(tokens).parse_compilation_unit()
    }

    /// `PROGRAM`ソースは`parse_compilation_unit`経由でも
    /// `CompilationUnit::Program`としてパースされること。
    #[test]
    fn parse_compilation_unit_dispatches_to_program() {
        let (unit, diags) = parse_unit_source("PROGRAM Foo; BEGIN END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        match unit {
            Some(wasd_ast::CompilationUnit::Program(program)) => {
                assert_eq!(program.name.name, "Foo");
            }
            other => panic!("expected a Program, got {other:?}"),
        }
    }

    /// `UNIT ... INTERFACE ... IMPLEMENTATION ... END.`が
    /// `CompilationUnit::Unit`としてパースされ、`INTERFACE`部の
    /// シグネチャと`IMPLEMENTATION`部の完全な宣言がそれぞれ正しい場所に
    /// 収まること。
    #[test]
    fn parses_unit_with_interface_and_implementation() {
        let src = r#"
            UNIT Greetings;

            INTERFACE

            USES Crt;

            CONST Greeting = 'H';

            PROCEDURE Hello;
            FUNCTION Add(a, b: INTEGER): INTEGER;

            IMPLEMENTATION

            PROCEDURE Hello;
            BEGIN
            END;

            FUNCTION Add(a, b: INTEGER): INTEGER;
            BEGIN
                Add := a + b
            END;

            END.
        "#;
        let (unit, diags) = parse_unit_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let unit = match unit {
            Some(wasd_ast::CompilationUnit::Unit(unit)) => unit,
            other => panic!("expected a Unit, got {other:?}"),
        };

        assert_eq!(unit.name.name, "Greetings");
        assert_eq!(unit.interface.uses.len(), 1);
        assert_eq!(unit.interface.uses[0].name, "Crt");
        assert_eq!(unit.interface.const_decls.len(), 1);
        assert_eq!(unit.interface.proc_signatures.len(), 1);
        assert_eq!(unit.interface.proc_signatures[0].name.name, "Hello");
        assert_eq!(unit.interface.func_signatures.len(), 1);
        assert_eq!(unit.interface.func_signatures[0].name.name, "Add");

        assert_eq!(unit.implementation.proc_decls.len(), 1);
        assert_eq!(unit.implementation.proc_decls[0].name.name, "Hello");
        assert_eq!(unit.implementation.func_decls.len(), 1);
        assert_eq!(unit.implementation.func_decls[0].name.name, "Add");
    }

    /// `PROGRAM`側の`USES`節も正しくパースされること。
    #[test]
    fn parses_program_with_uses_clause() {
        let (program, diags) = parse_source("PROGRAM Foo; USES Crt, Sysutils; BEGIN END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");
        assert_eq!(program.uses.len(), 2);
        assert_eq!(program.uses[0].name, "Crt");
        assert_eq!(program.uses[1].name, "Sysutils");
    }

    /// `CASE`文の`OTHERWISE`句がパースされ、`branches`とは別に
    /// `otherwise`へ格納されること。
    #[test]
    fn parses_case_statement_with_otherwise_clause() {
        let (program, diags) = parse_source(
            "PROGRAM Foo; VAR x, y: INTEGER; BEGIN CASE x OF 1: y := 1 OTHERWISE y := 2 END END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Case {
                branches,
                otherwise,
                ..
            } => {
                assert_eq!(branches.len(), 1);
                match otherwise.as_deref() {
                    Some(Statement::Assignment { target, .. }) => {
                        assert!(matches!(target, Expr::Identifier(id) if id.name == "y"));
                    }
                    other => panic!("expected an OTHERWISE assignment, got {other:?}"),
                }
            }
            other => panic!("expected a Case statement, got {other:?}"),
        }
    }

    /// `OTHERWISE`句を持たない`CASE`文では`otherwise`が`None`のままである
    /// こと（既存の挙動が壊れていないことのリグレッション確認）。
    #[test]
    fn case_statement_without_otherwise_has_none() {
        let (program, diags) = parse_source(
            "PROGRAM Foo; VAR x: INTEGER; BEGIN CASE x OF 1: x := 1 END END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Case { otherwise, .. } => assert!(otherwise.is_none()),
            other => panic!("expected a Case statement, got {other:?}"),
        }
    }

    /// `$FF`のような16進数リテラルが`Expr::HexIntLiteral`としてパースされ、
    /// 値が正しくデコードされること。
    #[test]
    fn parses_hex_literal_expression() {
        let (program, diags) =
            parse_source("PROGRAM Foo; VAR x: INTEGER; BEGIN x := $FF END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Assignment { value, .. } => {
                assert!(matches!(value, Expr::HexIntLiteral(0xFF, _)));
            }
            other => panic!("expected an Assignment, got {other:?}"),
        }
    }

    /// `STRING[n]`型が`TypeExpr::StringN`としてパースされること。
    #[test]
    fn parses_string_n_type() {
        let (program, diags) = parse_source("PROGRAM Foo; VAR s: STRING[10]; BEGIN END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        assert_eq!(program.var_decls.len(), 1);
        assert!(matches!(program.var_decls[0].ty, TypeExpr::StringN(10, _)));
    }

    /// コンパイラディレクティブが文の並びの中で
    /// `Statement::CompilerDirective`としてパースされること。
    #[test]
    fn parses_compiler_directive_statement() {
        let (program, diags) =
            parse_source("PROGRAM Foo; BEGIN (*$I foo.pas*) END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        assert_eq!(program.body.statements.len(), 1);
        match &program.body.statements[0] {
            Statement::CompilerDirective { name, args, .. } => {
                assert_eq!(name, "I");
                assert_eq!(args, "foo.pas");
            }
            other => panic!("expected a CompilerDirective statement, got {other:?}"),
        }
    }

    // ------------------------------------------------------------------
    // 配列・レコード・ポインタ型（Step 9）
    // ------------------------------------------------------------------

    /// `ARRAY [1..10] OF INTEGER`が`TypeExpr::Array`としてパースされること。
    #[test]
    fn parses_array_type() {
        let (program, diags) =
            parse_source("PROGRAM Foo; VAR a: ARRAY [1..10] OF INTEGER; BEGIN END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.var_decls[0].ty {
            TypeExpr::Array {
                index_type,
                element_type,
                packed,
                ..
            } => {
                assert!(!packed);
                assert!(matches!(
                    index_type.as_ref(),
                    TypeExpr::Subrange {
                        low: Literal::Int(1, _),
                        high: Literal::Int(10, _),
                        ..
                    }
                ));
                assert!(matches!(element_type.as_ref(), TypeExpr::Integer(_)));
            }
            other => panic!("expected an Array type, got {other:?}"),
        }
    }

    /// `PACKED ARRAY [0..255] OF CHAR`が`packed: true`でパースされること。
    #[test]
    fn parses_packed_array_type() {
        let (program, diags) =
            parse_source("PROGRAM Foo; VAR a: PACKED ARRAY [0..255] OF CHAR; BEGIN END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.var_decls[0].ty {
            TypeExpr::Array { packed, .. } => assert!(*packed),
            other => panic!("expected an Array type, got {other:?}"),
        }
    }

    /// `ARRAY [1..10, 1..20] OF INTEGER`が`ARRAY [1..10] OF ARRAY [1..20] OF
    /// INTEGER`と同じネストした`Array`に展開されること。
    #[test]
    fn multi_dimensional_array_desugars_to_nested_array() {
        let (program, diags) =
            parse_source("PROGRAM Foo; VAR a: ARRAY [1..10, 1..20] OF INTEGER; BEGIN END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.var_decls[0].ty {
            TypeExpr::Array {
                index_type,
                element_type,
                ..
            } => {
                assert!(matches!(
                    index_type.as_ref(),
                    TypeExpr::Subrange {
                        low: Literal::Int(1, _),
                        high: Literal::Int(10, _),
                        ..
                    }
                ));
                match element_type.as_ref() {
                    TypeExpr::Array {
                        index_type,
                        element_type,
                        ..
                    } => {
                        assert!(matches!(
                            index_type.as_ref(),
                            TypeExpr::Subrange {
                                low: Literal::Int(1, _),
                                high: Literal::Int(20, _),
                                ..
                            }
                        ));
                        assert!(matches!(element_type.as_ref(), TypeExpr::Integer(_)));
                    }
                    other => panic!("expected a nested Array type, got {other:?}"),
                }
            }
            other => panic!("expected an Array type, got {other:?}"),
        }
    }

    /// `arr[i]`が`Expr::IndexAccess`としてパースされ、`arr[i, j]`が
    /// `arr[i][j]`と同じネストした`IndexAccess`に展開されること。
    #[test]
    fn parses_index_access_expression_and_multi_index() {
        let (program, diags) = parse_source(
            "PROGRAM Foo; VAR a: ARRAY [1..10, 1..10] OF INTEGER; x: INTEGER; BEGIN x := a[1, 2] END.",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Assignment { value, .. } => match value {
                Expr::IndexAccess { array, index, .. } => {
                    assert!(matches!(index.as_ref(), Expr::IntLiteral(2, _)));
                    match array.as_ref() {
                        Expr::IndexAccess { array, index, .. } => {
                            assert!(matches!(index.as_ref(), Expr::IntLiteral(1, _)));
                            assert!(matches!(array.as_ref(), Expr::Identifier(id) if id.name == "a"));
                        }
                        other => panic!("expected a nested IndexAccess, got {other:?}"),
                    }
                }
                other => panic!("expected an IndexAccess, got {other:?}"),
            },
            other => panic!("expected an Assignment, got {other:?}"),
        }
    }

    /// `arr[i] := expr`が`target`に`Expr::IndexAccess`を持つ`Assignment`として
    /// パースされること。
    #[test]
    fn parses_assignment_to_array_element() {
        let (program, diags) =
            parse_source("PROGRAM Foo; VAR a: ARRAY [1..10] OF INTEGER; BEGIN a[1] := 42 END.");
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Assignment { target, value, .. } => {
                assert!(matches!(target, Expr::IndexAccess { .. }));
                assert!(matches!(value, Expr::IntLiteral(42, _)));
            }
            other => panic!("expected an Assignment, got {other:?}"),
        }
    }

    /// `RECORD ... END`型宣言が`TypeExpr::Record`としてパースされ、
    /// `rec.field`が`Expr::FieldAccess`としてパースされること。
    #[test]
    fn parses_record_type_and_field_access() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                Point = RECORD x, y: INTEGER END;
            VAR
                p: Point;
                n: INTEGER;
            BEGIN
                p.x := 1;
                n := p.y
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        assert_eq!(program.type_decls.len(), 1);
        assert_eq!(program.type_decls[0].name.name, "Point");
        match &program.type_decls[0].ty {
            TypeExpr::Record { fields, packed, .. } => {
                assert!(!packed);
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].names.len(), 2);
            }
            other => panic!("expected a Record type, got {other:?}"),
        }

        match &program.body.statements[0] {
            Statement::Assignment { target, .. } => match target {
                Expr::FieldAccess { record, field, .. } => {
                    assert_eq!(field.name, "x");
                    assert!(matches!(record.as_ref(), Expr::Identifier(id) if id.name == "p"));
                }
                other => panic!("expected a FieldAccess, got {other:?}"),
            },
            other => panic!("expected an Assignment, got {other:?}"),
        }
    }

    /// ポインタ型`^Node`と、`TYPE`セクション内での前方参照
    /// （`PNode = ^Node;`が`Node`の宣言より前に現れる）が構文的に
    /// 受理されること。名前解決自体は`wasd-sema`の責務なので、ここでは
    /// パーサーが`TypeExpr::Pointer(TypeExpr::Named("Node"))`を構築する
    /// ことだけを確認する。
    #[test]
    fn parses_pointer_type_with_forward_reference() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                PNode = ^Node;
                Node = RECORD
                    value: INTEGER;
                    next: PNode
                END;
            VAR
                head: PNode;
            BEGIN
                head := NIL
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        assert_eq!(program.type_decls.len(), 2);
        match &program.type_decls[0].ty {
            TypeExpr::Pointer(inner, _) => {
                assert!(matches!(inner.as_ref(), TypeExpr::Named(id) if id.name == "Node"));
            }
            other => panic!("expected a Pointer type, got {other:?}"),
        }

        match &program.body.statements[0] {
            Statement::Assignment { value, .. } => {
                assert!(matches!(value, Expr::NilLiteral(_)));
            }
            other => panic!("expected an Assignment, got {other:?}"),
        }
    }

    /// `p^`が`Expr::Deref`としてパースされ、`p^.field`のように
    /// デリファレンス後のフィールドアクセスも組み合わせられること。
    #[test]
    fn parses_deref_and_deref_field_access() {
        let src = r#"
            PROGRAM Foo;
            TYPE
                Node = RECORD value: INTEGER END;
                PNode = ^Node;
            VAR
                p: PNode;
                n: INTEGER;
            BEGIN
                p^.value := 1;
                n := p^.value
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        let program = program.expect("should parse a Program");

        match &program.body.statements[0] {
            Statement::Assignment { target, .. } => match target {
                Expr::FieldAccess { record, field, .. } => {
                    assert_eq!(field.name, "value");
                    assert!(matches!(record.as_ref(), Expr::Deref { .. }));
                }
                other => panic!("expected a FieldAccess over a Deref, got {other:?}"),
            },
            other => panic!("expected an Assignment, got {other:?}"),
        }
    }
}
