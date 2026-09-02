//! 宣言（`PROGRAM`/`VAR`/`CONST`/`TYPE`/`PROCEDURE`/`FUNCTION`など）のASTノード。
//!
//! 今回のスコープ: `PROGRAM`ヘッダ、単一の`BEGIN...END.`ブロック、
//! `VAR`セクション（組み込み型 + UCSD拡張の`STRING[n]` + 配列・レコード・
//! ポインタ型）、`CONST`セクション（リテラル値のみ）、`TYPE`セクション
//! （配列・レコード・ポインタ型を含む型宣言。`PROGRAM`直下と`UNIT`の
//! `INTERFACE`部のみに対応し、`PROCEDURE`/`FUNCTION`本体内のローカル
//! `TYPE`宣言は今回もまだ未対応。既存の`CONST`のローカル宣言と同様の
//! スコープ制限）、`PROCEDURE`/`FUNCTION`宣言（ローカル`VAR`宣言を含む）、
//! UCSD拡張の`UNIT`/`INTERFACE`/`IMPLEMENTATION`/`USES`。
//!
//! `Program`と`Unit`はいずれも「コンパイル単位」であり、[`CompilationUnit`]
//! でまとめて扱える。`wasd-parser`のエントリポイントは先頭トークンが
//! `PROGRAM`か`UNIT`かを見て、どちらをパースするかを決める。

use crate::expr::Literal;
use crate::ident::Identifier;
use crate::span::Span;
use crate::stmt::Block;

/// トップレベルのコンパイル単位。1つのソースファイルは`PROGRAM`宣言か
/// `UNIT`宣言のどちらか一方を持つ。
#[derive(Debug, Clone, PartialEq)]
pub enum CompilationUnit {
    Program(Program),
    Unit(Unit),
}

/// `PROGRAM <identifier>; ... BEGIN ... END.` 全体。
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub name: Identifier,
    /// UCSD拡張: `USES`節で参照する他`UNIT`名の並び。
    ///
    /// クロスファイル・クロスUNITなシンボル解決（`USES`で参照した`UNIT`の
    /// 公開シンボルをどう取り込むか）は今回のスコープ外（Step 7のタスク文書
    /// 参照）。ここでは構文的に参照名の並びを保持するだけで、名前解決は行わない。
    pub uses: Vec<Identifier>,
    pub const_decls: Vec<ConstDecl>,
    /// `TYPE`セクション。配列・レコード・ポインタ型の宣言（`TYPE Name = ...;`）
    /// を持つ。宣言順序が意味を持つ点に注意: `wasd-sema`はポインタ型の指す先
    /// レコードだけを対象とした前方参照（同じ`TYPE`セクション内で後から
    /// 宣言されるレコード型をポインタが指すこと）を許可するため、宣言順に
    /// 依存した解決を行う（`wasd-sema`の型解決ロジックのドキュメント参照）。
    pub type_decls: Vec<TypeDecl>,
    pub var_decls: Vec<VarDecl>,
    pub proc_decls: Vec<ProcDecl>,
    pub func_decls: Vec<FuncDecl>,
    pub body: Block,
    pub span: Span,
}

/// UCSD拡張: `UNIT name; INTERFACE ... IMPLEMENTATION ... END.` 全体。
///
/// # CONFIRMED: UNIT構文の基本構造
///
/// 2026-09-01の一次資料調査（リポジトリのUCSD Pascal一次資料調査メモ参照）
/// により、以下の基本構造が確認できた:
///
/// - `INTERFACE`部には実装（本体）を書いてはならず、宣言・シグネチャのみを
///   持つ。本実装の[`InterfaceSection`]（シグネチャのみ保持）はこの設計に
///   合致する。
/// - `IMPLEMENTATION`部に実際の`PROCEDURE`/`FUNCTION`の本体を書く。
/// - `UNIT`は単体では実行できないが、それ以外は`PROGRAM`と類似した構造
///   （定数・型・変数・ルーチンの定義）を持つ。
/// - `USES`節はコンパイラに対し、指定した`UNIT`のコードを取り込み、その
///   `UNIT`の`INTERFACE`部で宣言された識別子を、あたかも自分のモジュールの
///   一部であるかのように利用可能にするよう指示する。
///
/// 出典: Wikibooks, "Pascal Programming/Units"
/// <https://en.wikibooks.org/wiki/Pascal_Programming/Units>（一次資料では
/// ないが、UCSD Pascalのunit機構の基本構造の確認に用いた）。より一次資料に
/// 近い *UCSD PASCAL I.5 Manual* (Version I.5, September 1978) Section
/// 2.2.21「UNITS」（目次上はp.156付近）にも該当の解説があるはずだが、OCR化
/// されたテキストの文字化けにより本文の直接確認はまだ取れていない。
///
/// # 未実装: UNIT初期化・終了処理（p-machineレベルでは存在を確認済み）
///
/// SofTech Microsystems, *UCSD p-System and UCSD Pascal Version IV: Internal
/// Architecture Guide* (First edition, March 1981)
/// <https://archive.org/details/UCSD_P-System_UCSD_PASCAL_Internal_Architecture_Guide>
/// により、p-machine内部では各`UNIT`がコンパイル単位ごとに「セグメント参照
/// リスト」を持ち、名前`'***'`の特別なセグメント参照を通じて初期化・終了
/// コードセクションが実行されることが確認できた。ホストプログラムを実行する
/// 前に、オペレーティングシステムは使用中の全`UNIT`のリストを構築し、その
/// リストを使ってホストプログラムの呼び出し前後に各`UNIT`の初期化・終了
/// セクションを実行する。
///
/// ただしこれはp-machine内部仕様レベルの確認であり、Pascal言語レベルで
/// どのような構文（`IMPLEMENTATION`部末尾の`BEGIN ... END.`など）で書くのか
/// はUsers' Manual該当章のOCR文字化けにより今回未確認（UNCONFIRMED）のまま
/// である。そのため本実装の[`ImplementationSection`]には対応するフィールド
/// （例: `init_body: Option<Block>`）を**まだ追加していない**。次のAST拡張
/// ステップで、正確な構文が確認でき次第の追加を検討すること。
///
/// # UNCONFIRMED: 残る未確認事項
///
/// - `IMPLEMENTATION`部だけに存在する非公開の`PROCEDURE`/`FUNCTION`
///   （`INTERFACE`部に現れない）が許可されるかどうかは、一次資料での明記が
///   見つかっていない（Wikibooks等の二次資料では一般的なPascal unit解説として
///   触れられている）。本実装では慣用的に許可されると仮定し、`INTERFACE`部の
///   `proc_signatures`/`func_signatures`と`IMPLEMENTATION`部の`proc_decls`/
///   `func_decls`との突き合わせ（本体を持つ宣言が対応するシグネチャを持つか
///   等）は一切行わない。
/// - `UNIT`間の循環参照が許可されるか禁止されるかは、これを明記した一次資料が
///   見つかっておらず未確認。今回はそもそも`USES`解決自体を実装しないため、
///   循環参照の検出も行わない。
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub name: Identifier,
    pub interface: InterfaceSection,
    pub implementation: ImplementationSection,
    pub span: Span,
}

/// `UNIT`の`INTERFACE`部。ここで宣言されたものが外部（`USES`節でこの
/// `UNIT`を参照する側）に公開される想定（実際の公開範囲の強制・
/// クロスファイル名前解決は今回のスコープ外）。
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceSection {
    pub uses: Vec<Identifier>,
    pub const_decls: Vec<ConstDecl>,
    pub type_decls: Vec<TypeDecl>,
    pub var_decls: Vec<VarDecl>,
    /// 本体を持たない`PROCEDURE`シグネチャのみ。
    pub proc_signatures: Vec<ProcSignature>,
    pub func_signatures: Vec<FuncSignature>,
    pub span: Span,
}

/// `UNIT`の`IMPLEMENTATION`部。`INTERFACE`部のシグネチャに対応する
/// 実際の本体（またはIMPLEMENTATION部だけに存在する非公開の宣言。
/// [`Unit`]のドキュメントのUNCONFIRMED項参照）を持つ。初期化・終了処理
/// （`'***'`セグメント参照。[`Unit`]のドキュメント参照）に対応するフィールド
/// はまだ持たない。
#[derive(Debug, Clone, PartialEq)]
pub struct ImplementationSection {
    pub proc_decls: Vec<ProcDecl>,
    pub func_decls: Vec<FuncDecl>,
    pub span: Span,
}

/// `INTERFACE`部に現れる、本体を持たない`PROCEDURE name(params);`宣言。
#[derive(Debug, Clone, PartialEq)]
pub struct ProcSignature {
    pub name: Identifier,
    pub params: Vec<ParamDecl>,
    pub span: Span,
}

/// `INTERFACE`部に現れる、本体を持たない`FUNCTION name(params): returnType;`宣言。
#[derive(Debug, Clone, PartialEq)]
pub struct FuncSignature {
    pub name: Identifier,
    pub params: Vec<ParamDecl>,
    pub return_type: TypeExpr,
    pub span: Span,
}

/// `VAR`セクション中の1グループ（`x, y: Integer;`のような識別子リストと型の組）。
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub names: Vec<Identifier>,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `CONST`セクション中の1宣言（`Pi = 3.14;`のような識別子とリテラル値の組）。
#[derive(Debug, Clone, PartialEq)]
pub struct ConstDecl {
    pub name: Identifier,
    pub value: Literal,
    pub span: Span,
}

/// `TYPE`セクション中の1宣言（`Name = TypeExpr;`）。
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub name: Identifier,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `RECORD`型中の1フィールドグループ（`field1, field2: INTEGER;`のような
/// 識別子リストと型の組。`VarDecl`と同じ形）。
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub names: Vec<Identifier>,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `PROCEDURE name(params); [VAR ...] BEGIN ... END;` 宣言。
#[derive(Debug, Clone, PartialEq)]
pub struct ProcDecl {
    pub name: Identifier,
    pub params: Vec<ParamDecl>,
    /// ローカル変数（`VAR`セクション）。
    pub var_decls: Vec<VarDecl>,
    pub body: Block,
    pub span: Span,
}

/// `FUNCTION name(params): returnType; [VAR ...] BEGIN ... END;` 宣言。
#[derive(Debug, Clone, PartialEq)]
pub struct FuncDecl {
    pub name: Identifier,
    pub params: Vec<ParamDecl>,
    pub return_type: TypeExpr,
    /// ローカル変数（`VAR`セクション）。
    pub var_decls: Vec<VarDecl>,
    pub body: Block,
    pub span: Span,
}

/// 仮引数リスト中の1引数。
///
/// `PROCEDURE P(VAR a, b: INTEGER; c: REAL)`のように、1つの
/// `formal-parameter-section`が複数の名前を共有することがあるが、
/// ここでは名前ごとに展開した1件を表す（`by_ref`/`ty`はグループ内の
/// 全員で共通の値を持つ）。
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub name: Identifier,
    pub ty: TypeExpr,
    /// `VAR`引数（参照渡し）かどうか。
    pub by_ref: bool,
    pub span: Span,
}

/// 型。組み込み型に加え、UCSD拡張の`STRING[n]`型、配列・レコード・ポインタ型、
/// および`TYPE`セクションで宣言された型名への参照を持つ。
///
/// `Expr`のリテラルバリアントと同様、型を書いたソース上の位置を
/// バリアントごとに`Span`として保持する（型名の綴りに対する診断のため）。
///
/// # 設計判断: 多次元配列は`Array`のネストとして表現する
///
/// `ARRAY [1..10, 1..20] OF INTEGER`は`ARRAY [1..10] OF ARRAY [1..20] OF
/// INTEGER`の糖衣構文として扱い、パーサーがASTレベルでネストした`Array`
/// （`element_type`が`Array`自身であるような入れ子構造）に展開する。
/// 複数次元を1つの`Array`バリアントに直接持たせる（`index_types: Vec<...>`
/// のような設計）よりも実装がシンプルであり、`wasd-sema`側の型検査
/// （添字1つに対して1回`Array`を剥がす）も再帰で自然に書けるため、この方針を
/// 採用した。
///
/// # 設計判断: `Named`（型名参照）を追加した理由
///
/// タスク仕様の`TypeExpr`草案には現れないが、`TYPE`セクションの導入に伴い、
/// 「まだ組み込み型として認識できない識別子」を型の位置に書けるようにする
/// 必要がある（`TYPE MyRecord = RECORD ... END; VAR x: MyRecord;`の
/// `MyRecord`、`TYPE PNode = ^Node;`の`Node`など）。パーサーは`Named`として
/// 構文的に受理するだけで、実際にその名前が`TYPE`宣言に存在するかどうかの
/// 解決は`wasd-sema`が行う（ポインタの指す先レコードに限り、同じ`TYPE`
/// セクション内での前方参照を許可する。`wasd-sema`の型解決ロジックの
/// ドキュメント参照）。
///
/// `#[non_exhaustive]`: 将来さらにバリアント（集合型など）を追加する際に、
/// 既存の`match`をワイルドカードなしで壊さないようにするため。
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum TypeExpr {
    Integer(Span),
    Real(Span),
    Boolean(Span),
    Char(Span),
    /// UCSD拡張: `STRING[n]`（`n`は最大長）。
    ///
    /// # Step 16: `n`の型を`u8`にした理由
    ///
    /// `n`（最大長）は0-255の範囲に制約される（[`crate::decl`]モジュール外、
    /// `crates/wasd-parser/src/parser.rs`の`parse_string_n_type`のドキュメント
    /// 参照）。この制約自体は、STRING[n]のメモリレイアウトが「先頭1バイト＝
    /// 長さ、続く最大`n`バイトが文字データ」（長さフィールドが1バイトである
    /// こと）を前提とした場合の論理的帰結であり、その前提自体はUCSD
    /// p-System固有の一次資料では確認できていない（`docs/research/
    /// ucsd-pascal-primary-sources.md`の「Step 16セッション」節、UNCONFIRMED
    /// 項目1参照）。一般的なPascal系実装の慣習としては広く確認されている
    /// レイアウトであり、この前提のもとでは`n`が`u8`に収まることは
    /// 一次資料の確認を要さない単純な算術的帰結（1バイトの長さフィールドが
    /// 表現できる最大値は255）である。
    StringN(u8, Span),
    /// `TYPE`セクションで宣言された型名への参照（未解決）。
    Named(Identifier),
    /// `ARRAY [low..high] OF element`（`PACKED`修飾を含む）。
    Array {
        index_type: Box<TypeExpr>,
        element_type: Box<TypeExpr>,
        packed: bool,
        span: Span,
    },
    /// 添字の範囲を表現するサブレンジ型（`low..high`）。今回のスコープでは
    /// `Array::index_type`としてのみ現れ、かつ`low`/`high`はいずれも
    /// `Literal::Int`のみをサポートする（`wasd-sema`のドキュメント参照）。
    Subrange {
        low: Literal,
        high: Literal,
        span: Span,
    },
    /// `RECORD field1, field2: T1; ... END`（`PACKED`修飾を含む）。
    ///
    /// `CASE tag OF ... END`形式のvariant partは今回のスコープ外
    /// （タスク文書参照）。
    Record {
        fields: Vec<FieldDecl>,
        packed: bool,
        span: Span,
    },
    /// `^T`（ポインタ型）。
    Pointer(Box<TypeExpr>, Span),
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Integer(span)
            | TypeExpr::Real(span)
            | TypeExpr::Boolean(span)
            | TypeExpr::Char(span)
            | TypeExpr::StringN(_, span) => *span,
            TypeExpr::Named(ident) => ident.span,
            TypeExpr::Array { span, .. } => *span,
            TypeExpr::Subrange { span, .. } => *span,
            TypeExpr::Record { span, .. } => *span,
            TypeExpr::Pointer(_, span) => *span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::Expr;
    use crate::stmt::Statement;

    fn ident(name: &str, s: Span) -> Identifier {
        Identifier::new(name, s)
    }

    /// `PROGRAM Foo; BEGIN END.`に相当する最小ASTを手で構築し、
    /// フィールドアクセスが期待通り動くこと。
    #[test]
    fn builds_minimal_program() {
        let program_span = Span::new(0, 21);
        let name_span = Span::new(8, 11);
        let body_span = Span::new(13, 21);

        let program = Program {
            name: ident("Foo", name_span),
            uses: vec![],
            const_decls: vec![],
            type_decls: vec![],
            var_decls: vec![],
            proc_decls: vec![],
            func_decls: vec![],
            body: Block {
                statements: vec![],
                span: body_span,
            },
            span: program_span,
        };

        assert_eq!(program.name.name, "Foo");
        assert_eq!(program.name.span, name_span);
        assert!(program.const_decls.is_empty());
        assert!(program.var_decls.is_empty());
        assert!(program.body.statements.is_empty());
        assert_eq!(program.span, program_span);
    }

    /// `VarDecl`/`ConstDecl`を含むASTを構築し、型が正しく組めること。
    #[test]
    fn builds_program_with_var_and_const_decls() {
        let s = Span::new(0, 1);

        let var_decl = VarDecl {
            names: vec![ident("x", s), ident("y", s)],
            ty: TypeExpr::Integer(s),
            span: s,
        };
        let const_decl = ConstDecl {
            name: ident("MaxScore", s),
            value: Literal::Real(3.5, s),
            span: s,
        };

        let program = Program {
            name: ident("Foo", s),
            uses: vec![],
            const_decls: vec![const_decl.clone()],
            type_decls: vec![],
            var_decls: vec![var_decl.clone()],
            proc_decls: vec![],
            func_decls: vec![],
            body: Block {
                statements: vec![],
                span: s,
            },
            span: s,
        };

        assert_eq!(program.var_decls[0].names.len(), 2);
        assert_eq!(program.var_decls[0].ty, TypeExpr::Integer(s));
        assert_eq!(program.const_decls[0].value, Literal::Real(3.5, s));
        assert_eq!(var_decl.ty.span(), s);
        assert_eq!(const_decl.value.span(), s);
    }

    /// 各ノードが`Span`を保持していること（代表的なノードで確認）。
    #[test]
    fn nodes_carry_spans() {
        let decl_span = Span::new(5, 20);
        let var_decl = VarDecl {
            names: vec![Identifier::new("x", Span::new(5, 6))],
            ty: TypeExpr::Boolean(Span::new(8, 15)),
            span: decl_span,
        };
        assert_eq!(var_decl.span, decl_span);
        assert_eq!(var_decl.names[0].span, Span::new(5, 6));
        assert_eq!(var_decl.ty.span(), Span::new(8, 15));

        let stmt = Statement::ProcCall {
            name: Identifier::new("WriteLn", Span::new(0, 7)),
            args: vec![Expr::Identifier(Identifier::new("x", Span::new(8, 9)))],
            span: Span::new(0, 10),
        };
        assert_eq!(stmt.span(), Span::new(0, 10));
    }
}
