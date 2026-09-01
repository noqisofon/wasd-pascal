//! 再帰下降パーサー本体。
//!
//! # スコープ
//!
//! `wasd-ast`の最小定義に一致させる:
//! - `PROGRAM <identifier>;` ヘッダ + 単一の`BEGIN...END.`ブロック
//! - `VAR`/`CONST`宣言（組み込み型`INTEGER`/`REAL`/`BOOLEAN`/`CHAR`のみ）
//! - 文: 代入、`IF...THEN...[ELSE...]`、`WHILE...DO...`、複合文`BEGIN...END`、
//!   手続き呼び出し
//! - 式: リテラル、識別子、二項演算、単項演算、括弧
//!
//! `PROCEDURE`/`FUNCTION`宣言、`FOR`/`REPEAT`/`CASE`文、配列・レコード型、
//! `UNIT`等のUCSD拡張構文はまだパースしない。レキサはこれらをトークンとして
//! 認識できるが、このパーサーはまだ対応する文法規則を持たないというだけの
//! 状態であり、遭遇した場合は構文エラーの`Diagnostic`を発する。
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
    BinOp, Block, ConstDecl, Diagnostic, Expr, Identifier, Literal, Program, Severity, Span,
    Statement, TypeExpr, UnOp, VarDecl,
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

        let mut const_decls = Vec::new();
        let mut var_decls = Vec::new();

        loop {
            match self.peek() {
                TokenKind::Const => const_decls.extend(self.parse_const_section()),
                TokenKind::Var => var_decls.extend(self.parse_var_section()),
                TokenKind::Procedure
                | TokenKind::Function
                | TokenKind::Type
                | TokenKind::Label
                | TokenKind::Unit
                | TokenKind::Uses => {
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

        let body = match self.peek() {
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
        };

        self.expect(&TokenKind::Dot, "'.'");

        let end = self.previous_span().end.max(start.end);
        let program = Program {
            name,
            const_decls,
            var_decls,
            body,
            span: Span::new(start.start, end),
        };

        (Some(program), std::mem::take(&mut self.diagnostics))
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

    fn parse_type(&mut self) -> TypeExpr {
        let span = self.peek_span();
        match self.peek().clone() {
            TokenKind::Identifier(name) => {
                self.advance();
                match name.to_ascii_lowercase().as_str() {
                    "integer" => TypeExpr::Integer(span),
                    "real" => TypeExpr::Real(span),
                    "boolean" => TypeExpr::Boolean(span),
                    "char" => TypeExpr::Char(span),
                    _ => {
                        self.error(
                            span,
                            format!(
                                "unknown type '{name}' (only INTEGER/REAL/BOOLEAN/CHAR are supported by this parser)"
                            ),
                        );
                        TypeExpr::Integer(span)
                    }
                }
            }
            other => {
                self.error(
                    span,
                    format!(
                        "expected a type name (INTEGER/REAL/BOOLEAN/CHAR), found {}",
                        describe(&other)
                    ),
                );
                if !matches!(other, TokenKind::Semicolon | TokenKind::Eof) {
                    self.advance();
                }
                TypeExpr::Integer(span)
            }
        }
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
            TokenKind::Identifier(_) => self.parse_assignment_or_call(),

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

            TokenKind::Procedure
            | TokenKind::Function
            | TokenKind::For
            | TokenKind::Repeat
            | TokenKind::Case
            | TokenKind::With
            | TokenKind::Goto
            | TokenKind::Label => {
                let span = self.peek_span();
                self.error(
                    span,
                    format!(
                        "{} is not supported by this parser yet (only assignment, IF/THEN/ELSE, WHILE/DO, compound statements, and procedure calls are supported)",
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

        let mut statements = Vec::new();
        loop {
            // 連続するセミコロン（空文）を読み飛ばす。
            while self.check(&TokenKind::Semicolon) {
                self.advance();
            }
            if self.check(&TokenKind::End) || self.is_eof() {
                break;
            }

            match self.parse_statement() {
                Some(stmt) => {
                    statements.push(stmt);

                    if self.check(&TokenKind::Semicolon) {
                        continue;
                    } else if self.check(&TokenKind::End) || self.is_eof() {
                        break;
                    } else {
                        let span = self.peek_span();
                        let found = describe(self.peek());
                        self.error(span, format!("expected ';' or 'END', found {found}"));
                        self.synchronize_to_statement_boundary();
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

        let end_span = self.expect_and_span(&TokenKind::End, "'END'");
        Some(Statement::Compound(Block {
            statements,
            span: Span::new(start.start, end_span.end),
        }))
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

    /// 代入文 `identifier := expr` と手続き呼び出し文
    /// `identifier(expr, ...)` / `identifier` は共に識別子から始まるため、
    /// ここでまとめて先読み分岐する。
    fn parse_assignment_or_call(&mut self) -> Option<Statement> {
        let name = self.parse_identifier("identifier");

        if self.check(&TokenKind::Assign) {
            self.advance();
            let value = self.parse_expr();
            let span = Span::new(name.span.start, value.span().end);
            Some(Statement::Assignment { target: name, value, span })
        } else if self.check(&TokenKind::LParen) {
            self.advance();
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
            let close = self.expect_and_span(&TokenKind::RParen, "')'");
            let span = Span::new(name.span.start, close.end.max(name.span.end));
            Some(Statement::ProcCall { name, args, span })
        } else {
            let span = name.span;
            Some(Statement::ProcCall { name, args: vec![], span })
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
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Expr {
        let span = self.peek_span();
        match self.peek().clone() {
            TokenKind::IntegerLiteral(v) => {
                self.advance();
                Expr::IntLiteral(v, span)
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
                match name.to_ascii_lowercase().as_str() {
                    "true" => Expr::BoolLiteral(true, span),
                    "false" => Expr::BoolLiteral(false, span),
                    _ => Expr::Identifier(Identifier::new(name, span)),
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
    fn reports_diagnostic_for_unsupported_for_statement_without_panicking() {
        let src = r#"
            PROGRAM Foo;
            VAR x: INTEGER;
            BEGIN
                FOR x := 1 TO 10 DO ;
                x := 1
            END.
        "#;
        let (program, diags) = parse_source(src);
        assert!(!diags.is_empty(), "expected at least one diagnostic for FOR");
        let program = program.expect("parser should still produce a Program despite the error");
        assert_eq!(program.body.statements.len(), 1);
        match &program.body.statements[0] {
            Statement::Assignment { value, .. } => {
                assert!(matches!(value, Expr::IntLiteral(1, _)));
            }
            other => panic!("expected the recovered assignment, got {other:?}"),
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
}
