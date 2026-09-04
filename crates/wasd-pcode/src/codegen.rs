//! ASTからp-code命令列を生成するコード生成器。
//!
//! # 今回のスコープ
//!
//! `INTEGER`/`BOOLEAN`型の変数・定数、算術演算（`+ - * DIV MOD`）、比較演算、
//! 論理演算（`AND OR NOT`）、代入文、`IF`/`WHILE`/`REPEAT UNTIL`/`FOR`に
//! よる制御構造、`BEGIN...END`の複合文、`PROGRAM ... BEGIN ... END.`
//! 全体構造、`PROGRAM`直下に宣言された`PROCEDURE`/`FUNCTION`の呼び出し
//! （Step 18時点では単一の値仮引数または`VAR`仮引数のみ。`INTEGER`/
//! `BOOLEAN`/`STRING[n]`。Step 21から任意個数の仮引数（型混在可）に一般化
//! （[`CodeGenerator::build_params`]/[`CodeGenerator::gen_call_args`]参照）。
//! Step 18から`FUNCTION`の戻り値も含む）、および組み込み手続き`WriteLn`
//! （引数0個または1個。`INTEGER`/`BOOLEAN`に加え、Step 15から文字列
//! リテラルの直接渡し（`WriteLn('...')`）も、Step 16から`STRING[n]`変数も
//! サポート。[`CodeGenerator::gen_writeln_call`]参照）を扱う。文字列
//! リテラルは`WriteLn`への直接渡し、および`STRING[n]`値引数として渡す
//! 場合（Step 18。[`CodeGenerator::gen_string_value_arg`]参照）以外の
//! 文脈では引き続きスコープ外（意味解析（`wasd-sema`）の段階で型エラーに
//! なるため、本クレートまで到達しない）。
//!
//! Step 19からは、UCSD拡張ではなく標準Pascalの配列型（`ARRAY [low..high]
//! OF element`）についても、`PROGRAM`直下のグローバル`VAR`宣言・要素が
//! `INTEGER`/`BOOLEAN`・1次元のみという条件付きでサポートする
//! （`STRING[n]`がStep 16でまずグローバル変数のみサポートされた前例を
//! 踏襲。[`ArrayKind`]のドキュメント参照）。添字アクセス（読み込み
//! `arr[i]`・代入`arr[i] := value`の両方）は、専用の新しいp-code
//! オペコードを追加せず、既存の[`UnconfirmedOp::Lda`]/[`UnconfirmedOp::Ind`]/
//! [`UnconfirmedOp::Sti`]と算術命令（`LDC`/`SBI`/`ADI`）の組み合わせだけで
//! 合成する（[`CodeGenerator::gen_array_element_address`]のドキュメント
//! 「設計判断」参照）。`PROCEDURE`/`FUNCTION`のローカル配列変数・配列型
//! 仮引数・多次元配列・`INTEGER`/`BOOLEAN`以外を要素とする配列・配列同士の
//! 丸ごと代入（`a := b`）・実行時の添字範囲チェックは、いずれも引き続き
//! 今回のスコープ外（`wasd-sema`のコンパイル時定数添字チェックのみで、
//! 実行時チェックは行わない方針とも歩調を合わせる。`crates/wasd-sema/src/
//! typeck.rs`の`infer_index_access_type`ドキュメント参照）。
//!
//! Step 20からは、`TYPE Name = RECORD field1, field2: T1; ... END;`という
//! `TYPE`宣言（レコード型のみ。他の種類の`TYPE`宣言——配列型・ポインタ型・
//! 単純な型名の別名等——は引き続きスコープ外）、およびそのレコード型（＋
//! `VAR`宣言に直接書かれた無名レコード型）を持つ`PROGRAM`直下のグローバル
//! `VAR`変数についてもサポートする。フィールドの読み書き
//! （`rec.field`、[`wasd_ast::Expr::FieldAccess`]）は、配列の添字アクセス
//! （実行時に決まる値なので`LDA`+算術命令によるアドレス計算が必要だった。
//! [`CodeGenerator::gen_array_element_address`]のドキュメント参照）とは
//! 異なり、フィールドのオフセットが常にコンパイル時定数であるため、
//! レコード変数のベースアドレスへオフセットを加算した絶対アドレスを
//! そのまま[`UnconfirmedOp::Lod`]/[`UnconfirmedOp::Str`]のアドレス
//! オペランドとして使うだけで完結する（[`CodeGenerator::resolve_field_access`]
//! のドキュメント「設計判断」参照）。フィールドの型はStep 19までに対応済みの
//! スカラー型（`INTEGER`/`BOOLEAN`/`STRING[n]`）のみで、配列・レコード・
//! ポインタを要素とする複合的なフィールド、レコードを配列要素にする・
//! `variant record`（`CASE tag OF ... END`）・`WITH`文・`PROCEDURE`/
//! `FUNCTION`内のローカルレコード変数はいずれも引き続きスコープ外
//! （`STRING[n]`/配列がまず`PROGRAM`直下のグローバル変数のみサポート
//! された前例を踏襲。[`RecordLayout`]のドキュメント参照）。
//!
//! Step 21からは、`PROCEDURE`/`FUNCTION`の仮引数リストを任意個数
//! （型混在可）に一般化し、さらに配列・レコード型も仮引数の型として
//! 許可する（[`CodeGenerator::build_params`]参照）。
//!
//! **UNCONFIRMED/TODO（既知の制限）**: 配列・レコードの値仮引数は、
//! 本来のPascalの値渡し意味論（呼び出し側でコピーを作成し、そのコピーの
//! アドレスを渡す）ではなく、コピーを作らず元の変数のアドレスをそのまま
//! 渡す**仮実装**である（`STRING[n]`の値仮引数がコピーを作るのとは対照的。
//! [`CodeGenerator::gen_string_value_arg`]参照）。そのため、呼び出された
//! 側での配列要素・レコードフィールドへの変更が呼び出し元にも反映されて
//! しまう（実質的に参照渡しのように振る舞う）という既知の制限がある。
//! これはタスク依頼で明示された意図的な単純化であり、正しい値渡し意味論
//! （コピー生成）は、将来の`VAR`パラメータの正式な構文・意味論の実装と
//! 合わせて別ステップで対応する予定（[`CodeGenerator::build_params`]/
//! [`CodeGenerator::gen_array_or_record_value_arg`]のドキュメント、
//! リポジトリの`README.md`「既知の制限」も参照）。
//!
//! `CASE`、`UNIT`、ポインタ型、`REAL`/`CHAR`型、`WriteLn`以外の組み込み
//! 手続き（`Write`/`Read`/`ReadLn`/`New`/`Dispose`）、複数引数の`WriteLn`は
//! 意味解析を通過済みのASTに含まれ得るが、本クレートの責務では**ない**。
//! 遭遇した場合はパニックせず、「未対応機能」の[`wasd_ast::Diagnostic`]を
//! 積んでコード生成のみ諦める（呼び出し元はレキサ・パーサー・意味解析と
//! 同様、`Result::Err`として診断の集合を受け取る）。
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
//!    （`P`は仮引数の**個数**——Step 21から複数仮引数を、宣言順に
//!    そのままパラメータ領域のオフセット順へ写す（[`CodeGenerator::build_params`]
//!    参照）。仮引数はどの型であっても常にちょうど1ワードを占める:
//!    `VAR`仮引数はアドレスを1ワードで格納し、それ以外の値仮引数のうち
//!    `INTEGER`/`BOOLEAN`は値そのものを1ワードで格納する。`STRING[n]`の
//!    値仮引数（Step 18）、および配列・レコードの値仮引数（Step 21。
//!    仮実装でコピーを作らずアドレスを渡す。[`CodeGenerator::build_params`]
//!    のドキュメント「UNCONFIRMED/TODO」参照）は、レコード・配列の値
//!    パラメータに関するStep 12のCONFIRMED済みの規則からの類推
//!    （[`CodeGenerator::gen_string_value_arg`]のドキュメント参照、
//!    UNCONFIRMED）により、`VAR`仮引数と同様にアドレスを1ワードで格納する。
//!    そのため`P`は単純に仮引数の個数と一致する）
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
    BinOp, Block, ConstDecl, Diagnostic, Expr, FieldDecl, ForDirection, FuncDecl, Identifier,
    Literal, ParamDecl, ProcDecl, Program, Severity, Span, Statement, TypeDecl, TypeExpr, UnOp,
    VarDecl,
};

use crate::builtin::{
    BUILTIN_WRITELN_BOOL, BUILTIN_WRITELN_INT, BUILTIN_WRITELN_NONE, BUILTIN_WRITELN_STRING,
    BUILTIN_WRITELN_STRVAR, KERNEL_SEGMENT,
};
use crate::ir::{Instruction, PCodeModule, RoutineMeta};
use crate::opcode::{Address, CodeAddress, ConfirmedOp, Level, Opcode, UnconfirmedOp};

/// 未確定のジャンプ先・呼び出し先を持つ命令のインデックス。
/// [`CodeGenerator::patch_jump`]に渡してバックパッチする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingJump(usize);

/// 式・変数・定数・`FUNCTION`戻り値の「種類」。本クレートのスコープは
/// `INTEGER`/`BOOLEAN`/`STRING[n]`（Step 16から）のみなので、
/// `wasd_sema::Type`のような一般的な型表現ではなく、この最小限のenumで
/// 済ませる。`WriteLn(expr)`が`BUILTIN_WRITELN_INT`/`BUILTIN_WRITELN_BOOL`/
/// `BUILTIN_WRITELN_STRVAR`のどれを呼ぶべきかを決めるためだけに使う
/// （[`CodeGenerator::infer_expr_kind`]参照）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Int,
    Bool,
    /// UCSD拡張`STRING[n]`（Step 16）。`u8`は宣言された最大長
    /// （`wasd_ast::TypeExpr::StringN`のドキュメント参照）。`INTEGER`/
    /// `BOOLEAN`と異なり1ワードの値ではなく、`1 + max_len`ワードを占める
    /// 複合的な変数を指す点に注意（[`word_size_of`]、
    /// [`CodeGenerator::declare_vars`]参照）。
    StringN(u8),
    /// 配列型（Step 19）。[`ArrayKind`]のドキュメント参照。
    Array(ArrayKind),
    /// レコード型（Step 20）。[`RecordKind`]のドキュメント参照。
    Record(RecordKind),
}

/// 配列要素の種類。本ステップのスコープでは`INTEGER`/`BOOLEAN`のみ
/// （いずれも1ワード）。[`ArrayKind::element`]参照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarValueKind {
    Int,
    Bool,
}

impl From<ScalarValueKind> for ValueKind {
    fn from(kind: ScalarValueKind) -> Self {
        match kind {
            ScalarValueKind::Int => ValueKind::Int,
            ScalarValueKind::Bool => ValueKind::Bool,
        }
    }
}

/// 配列型`ARRAY [low..high] OF element`の中身（Step 19）。
///
/// # スコープ: 1次元・`INTEGER`/`BOOLEAN`要素・`PROGRAM`直下のグローバル変数のみ
///
/// `STRING[n]`がStep 16でまず`PROGRAM`直下のグローバル変数のみサポート
/// されたのと同じ前例を踏襲する（[`CodeGenerator::build_locals`]・
/// [`CodeGenerator::build_params`]は引き続き配列型を拒否する。
/// [`CodeGenerator::declare_array_vars`]のドキュメント参照）。
/// 多次元配列（`element`がさらに配列であるケース）・`RECORD`/`STRING[n]`/
/// ポインタ型を要素とする配列は、いずれも今回のスコープ外
/// （[`CodeGenerator::declare_array_vars`]参照）。
///
/// `low`/`high`は`wasd_ast::Literal::Int`と同じ`i64`（`wasd-sema`の
/// `ArrayType`と同じ表現）。この時点（p-code生成はwasd-sema成功後にのみ
/// 走る。`crates/wasd-driver/src/pcode.rs`のドキュメント参照）では
/// `low <= high`は既に`wasd-sema`が保証済み
/// （`crates/wasd-sema/src/typeck.rs`の`eval_subrange_bounds`参照）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArrayKind {
    low: i64,
    high: i64,
    element: ScalarValueKind,
}

impl ArrayKind {
    /// 配列全体が占めるワード数（`high - low + 1`、要素はいずれも1ワード）。
    fn element_count(&self) -> i64 {
        self.high - self.low + 1
    }
}

/// [`CodeGenerator::record_layouts`]中の1件を指すインデックス（Step 20）。
///
/// `ValueKind`（延いては[`VarSlot`]/[`FrameSlot`]）が`Copy`であり続ける
/// ために、フィールド一覧そのもの（`Vec`で可変長）は`ValueKind`へ直接
/// 埋め込まず、この軽量なインデックス経由で[`CodeGenerator::record_layouts`]
/// を引く設計にしてある（[`crate::codegen`]モジュールの`string_pool`と
/// 同じ「インデックスで間接参照する」パターン）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecordId(usize);

/// レコード型`ValueKind::Record`が持つ情報（Step 20）。
///
/// `total_words`は[`RecordLayout::total_words`]と同じ値をこの`Copy`な
/// 構造体自身にも複製して持つ（[`word_size_of`]が`&CodeGenerator`を
/// 経由せず`ValueKind`単体から占有ワード数を求められるようにするため。
/// [`ArrayKind::element_count`]が`low`/`high`から自己完結的にワード数を
/// 求められるのと同じ理由）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecordKind {
    id: RecordId,
    total_words: u16,
}

/// レコード型1件のうち、フィールド1つのレイアウト（Step 20）。
#[derive(Debug, Clone)]
struct RecordFieldLayout {
    /// 正規化済み（小文字化済み）のフィールド名。
    name: String,
    /// レコード先頭からのオフセット（ワード単位）。
    offset: u16,
    kind: ValueKind,
}

/// レコード型1件のメモリレイアウト（Step 20）。
///
/// # 設計判断: フィールドオフセットは宣言順の単純な逐次積み上げ（UNCONFIRMED）
///
/// タスク依頼の指示、および一次資料の確認結果に基づく。このセッションでも
/// 改めて一次資料（SofTech Microsystems, *UCSD p-System and UCSD Pascal
/// Version IV: Internal Architecture Guide*）へのアクセスを`WebFetch`で
/// 試みたが、`archive.org`は本セッションのサンドボックスのネットワーク
/// 経路（agent proxy）で引き続き`EGRESS_BLOCKED`だった（`docs/research/
/// ucsd-pascal-primary-sources.md`参照。同ドキュメントにもレコード型の
/// メモリレイアウトに関する記述はない）。そのため、レコード型のメモリ
/// レイアウト（ワード境界アラインメント・パディングの要否を含む）は
/// **一次資料で確認できていない（UNCONFIRMED）**。本実装は「フィールド1の
/// オフセットは0、フィールド2はフィールド1のサイズ分後ろ」という単純な
/// 逐次積み上げを採用し、`packed`修飾の有無で挙動を変えない（タスク依頼が
/// 明示的に指示する簡略化）。
///
/// # スコープ: フィールドは`INTEGER`/`BOOLEAN`/`STRING[n]`のみ
///
/// 配列・レコード・ポインタを要素とするフィールド（複合的なフィールド）は
/// 今回のスコープ外（[`CodeGenerator::resolve_record_field_kind`]参照）。
#[derive(Debug, Clone)]
struct RecordLayout {
    fields: Vec<RecordFieldLayout>,
    /// レコード全体が占めるワード数（全フィールドのワード数の合計）。
    total_words: u16,
}

/// [`CodeGenerator::resolve_record_type_expr`]の結果（Step 20）。
enum RecordResolution {
    /// `TypeExpr`はレコード型ではない（他の型として解決を続けてよい）。
    NotRecord,
    /// レコード型として解決できた。
    Valid(RecordId),
    /// レコード型として解決しようとしたが、フィールドの型・サイズ等で
    /// エラーがあった（診断は既に発行済みなので、呼び出し元は追加の
    /// 診断を出さずスキップしてよい）。
    Invalid,
}

/// `TypeExpr`をこのクレートのスコープが対応する[`ValueKind`]へ変換する。
/// `INTEGER`/`BOOLEAN`/`STRING[n]`以外（このクレートのスコープ外の型）は
/// `None`。
///
/// `TypeExpr::Array`は意図的にここでは扱わない: 配列は`STRING[n]`と同様
/// （[`ArrayKind`]のドキュメント参照）`PROGRAM`直下のグローバル変数
/// でのみサポートするため、専用の[`CodeGenerator::declare_array_vars`]で
/// 個別に処理する。ここで`None`を返すことで、`PROCEDURE`/`FUNCTION`の
/// ローカル変数・仮引数の位置に現れた配列型は（従来通り）
/// [`describe_type`]を使った汎用の「スコープ外」診断に自然に流れる。
fn value_kind_of(ty: &TypeExpr) -> Option<ValueKind> {
    match ty {
        TypeExpr::Integer(_) => Some(ValueKind::Int),
        TypeExpr::Boolean(_) => Some(ValueKind::Bool),
        TypeExpr::StringN(n, _) => Some(ValueKind::StringN(*n)),
        _ => None,
    }
}

/// [`ValueKind`]の値が占めるワード数。`INTEGER`/`BOOLEAN`は1ワード、
/// `STRING[n]`は`1 + n`ワード（先頭1ワードが長さ、続く`n`ワードが
/// 文字データ。[`crate::builtin::BUILTIN_WRITELN_STRVAR`]のドキュメント
/// 「メモリレイアウト」参照）、配列は`high - low + 1`ワード
/// （[`ArrayKind::element_count`]参照）。
///
/// 配列については現状[`CodeGenerator::declare_array_vars`]がこの関数を
/// 経由せず直接ワード数を求めている（16bitワード数に収まるかどうかの
/// 診断を出す必要があるため）。ここでの`Array`腕は`ValueKind`の
/// 網羅性のために存在し、想定される呼び出しでは実際には使われない。
///
/// レコード（Step 20）は`RecordKind::total_words`（[`RecordLayout::total_words`]
/// の複製）をそのまま返す（[`RecordKind`]のドキュメント参照）。
fn word_size_of(kind: ValueKind) -> u16 {
    match kind {
        ValueKind::Int | ValueKind::Bool => 1,
        ValueKind::StringN(max_len) => 1 + max_len as u16,
        ValueKind::Array(arr) => u16::try_from(arr.element_count()).unwrap_or(u16::MAX),
        ValueKind::Record(rec) => rec.total_words,
    }
}

/// グローバル変数1件の情報。`address`は`STRING[n]`の場合、先頭ワード
/// （長さ）のアドレスを指す（[`word_size_of`]参照）。
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
///
/// `by_ref`と`indirect`は別の概念である点に注意:
/// - `by_ref`: 仮引数が`VAR`として宣言されたか（呼び出し元の変数への
///   参照渡しかどうか）。[`CodeGenerator::gen_call_args`]が呼び出し側の
///   引数の積み方（値を評価するか、アドレスを積むか）を選ぶために使う。
/// - `indirect`: このスロットに物理的に格納されているのが値そのものでは
///   なく**アドレス**であるかどうか。`by_ref`な仮引数（型を問わず）に加え、
///   `STRING[n]`の値仮引数（Step 18。`by_ref`は偽）でも真になる
///   （[`CodeGenerator::gen_string_value_arg`]のドキュメント参照:
///   レコード・配列の値パラメータに関するStep 12のCONFIRMED済みの規則
///   からの類推、UNCONFIRMED）。ロード/ストア/アドレス取得
///   （[`CodeGenerator::gen_load_resolved`]等）はこの`indirect`だけを見る。
#[derive(Debug, Clone, Copy)]
struct FrameSlot {
    address: Address,
    by_ref: bool,
    indirect: bool,
    kind: ValueKind,
}

/// 識別子1つの参照先が解決した結果。ロード/ストア/アドレス取得の
/// いずれの操作にも必要な情報（レベル差・アドレス・スロットがアドレスを
/// 格納しているかどうか）をまとめて持つ。
///
/// # `offset`フィールド（Step 21から）
///
/// `indirect`な参照（`VAR`仮引数、`STRING[n]`/配列/レコードの値仮引数）を
/// 経由してレコードのフィールドへアクセスする場合、フィールドのオフセット
/// はコンパイル時定数ではあるものの、`address`スロットに実際に格納されて
/// いるのは「レコード本体のアドレス」であって「レコード本体そのもの」
/// ではないため、単純に`address`へオフセットを足し込むことができない
/// （`indirect`でない場合との違い。[`CodeGenerator::resolve_field_access`]
/// のドキュメント参照）。そこで、`indirect`な参照については「スロットから
/// 読んだアドレスへ、実行時に`offset`ワード分を加算してから間接
/// アクセスする」という追加の情報として`offset`を持たせる
/// （[`CodeGenerator::gen_load_resolved`]/[`CodeGenerator::gen_store_resolved`]/
/// [`CodeGenerator::gen_address_of_resolved`]参照）。`indirect`が偽の場合、
/// オフセットは常に`address`へ静的に加算済みなので`offset`は常に`0`。
#[derive(Debug, Clone, Copy)]
struct ResolvedVar {
    level: Level,
    address: Address,
    /// [`FrameSlot::indirect`]参照。
    indirect: bool,
    /// `indirect`な参照でのみ意味を持つ、実行時に加算する追加オフセット
    /// （ワード単位）。`indirect`が偽の場合は常に`0`。
    offset: u16,
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
    /// レコード型のレイアウトプール（Step 20）。`ValueKind::Record`が
    /// 持つ[`RecordId`]でここを引く（[`RecordId`]のドキュメント参照）。
    record_layouts: Vec<RecordLayout>,
    /// `TYPE`セクションで宣言されたレコード型の名前(小文字正規化済み) ->
    /// [`RecordId`]（Step 20）。`TypeExpr::Named`が指す型がレコード型で
    /// あるかどうかの解決に使う（[`Self::resolve_record_type_expr`]参照）。
    record_type_names: HashMap<String, RecordId>,
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
        self.record_layouts.clear();
        self.record_type_names.clear();

        if !program.uses.is_empty() {
            self.error(
                program.span,
                "cross-unit code generation ('USES') is out of scope for this step's minimal \
                 codegen",
            );
        }

        self.declare_consts(&program.const_decls);
        self.declare_type_decls(&program.type_decls);
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
            if matches!(decl.ty, TypeExpr::Array { .. }) {
                self.declare_array_vars(decl);
                continue;
            }
            match self.resolve_record_type_expr(&decl.ty) {
                RecordResolution::Valid(id) => {
                    self.declare_record_vars(decl, id);
                    continue;
                }
                // フィールド型・サイズのエラーは`resolve_record_type_expr`が
                // 既に報告済みなので、ここでは追加の診断を出さずスキップする。
                RecordResolution::Invalid => continue,
                RecordResolution::NotRecord => {}
            }
            let Some(kind) = value_kind_of(&decl.ty) else {
                self.error(
                    decl.ty.span(),
                    format!(
                        "VAR type '{}' is out of scope for this step's minimal codegen (only \
                         INTEGER/BOOLEAN/STRING[n]/ARRAY/RECORD are supported)",
                        describe_type(&decl.ty)
                    ),
                );
                continue;
            };
            for name in &decl.names {
                let address = self.alloc_words(word_size_of(kind));
                self.vars
                    .insert(normalize(&name.name), VarSlot { address, kind });
            }
        }
    }

    /// `TYPE`セクション（Step 20）。`TYPE Name = RECORD ... END;`のみを
    /// サポートし、[`Self::record_type_names`]へ登録する（そのレコード型を
    /// 使う`VAR`宣言が[`Self::resolve_record_type_expr`]経由で
    /// `TypeExpr::Named`を解決できるようにするため）。それ以外の種類の
    /// `TYPE`宣言（配列型・ポインタ型・単純な型名の別名等）は引き続き
    /// スコープ外としてエラー報告のみ行う（従来、`program.type_decls`が
    /// 空でなければ即座にエラーにしていたのを、レコード型のみ許可する形に
    /// 緩和した）。
    fn declare_type_decls(&mut self, type_decls: &[TypeDecl]) {
        for decl in type_decls {
            match &decl.ty {
                TypeExpr::Record { fields, span, .. } => {
                    if let Some(layout) = self.build_record_layout(fields, *span) {
                        let id = RecordId(self.record_layouts.len());
                        self.record_layouts.push(layout);
                        self.record_type_names
                            .insert(normalize(&decl.name.name), id);
                    }
                }
                other => {
                    self.error(
                        other.span(),
                        format!(
                            "TYPE '{}' = {}: only 'TYPE Name = RECORD ... END;' is supported by \
                             this step's minimal codegen",
                            decl.name.name,
                            describe_type(other)
                        ),
                    );
                }
            }
        }
    }

    /// `RECORD`型（`TYPE`宣言経由・`VAR`宣言中の無名レコードのいずれも）の
    /// フィールド一覧から[`RecordLayout`]を組み立てる（Step 20。設計判断は
    /// [`RecordLayout`]のドキュメント参照）。
    ///
    /// 重複フィールド名の検出は行わない: `wasd-sema`が既に検出済みであり
    /// （`crates/wasd-sema/src/typeck.rs`の`resolve_record_fields_into`
    /// 参照）、本クレートは意味解析成功後のASTのみを受け取る前提
    /// （[`crate::codegen`]モジュールドキュメント参照）。
    fn build_record_layout(
        &mut self,
        fields: &[FieldDecl],
        record_span: Span,
    ) -> Option<RecordLayout> {
        let mut layout_fields = Vec::new();
        // `u32`で積み上げ、最後に`u16`（p-machineの1ワードアドレス空間）へ
        // 収まるか確認する（`Self::declare_array_vars`の要素数チェックと
        // 同じ方針）。
        let mut offset: u32 = 0;
        let mut ok = true;
        for field in fields {
            let Some(kind) = self.resolve_record_field_kind(&field.ty) else {
                ok = false;
                continue;
            };
            let words = u32::from(word_size_of(kind));
            for name in &field.names {
                match u16::try_from(offset) {
                    Ok(field_offset) => {
                        layout_fields.push(RecordFieldLayout {
                            name: normalize(&name.name),
                            offset: field_offset,
                            kind,
                        });
                    }
                    Err(_) => {
                        self.error(
                            name.span,
                            format!(
                                "record field '{}' does not fit in this step's minimal codegen \
                                 (offset exceeds a 16-bit word count)",
                                name.name
                            ),
                        );
                        ok = false;
                    }
                }
                offset += words;
            }
        }
        if !ok {
            return None;
        }
        match u16::try_from(offset) {
            Ok(total_words) => Some(RecordLayout {
                fields: layout_fields,
                total_words,
            }),
            Err(_) => {
                self.error(
                    record_span,
                    "record has too many fields for this step's minimal codegen (total size \
                     does not fit in a 16-bit word count)",
                );
                None
            }
        }
    }

    /// レコードフィールドの型を、本クレートのスコープが対応する
    /// [`ValueKind`]へ解決する（Step 20）。フィールドの型はStep 19までに
    /// 対応済みのスカラー型（`INTEGER`/`BOOLEAN`/`STRING[n]`）のみ
    /// サポートする（タスク文書の「今回のスコープ」参照）。配列・レコード・
    /// ポインタをフィールドとする複合構造はいずれもスコープ外。
    fn resolve_record_field_kind(&mut self, ty: &TypeExpr) -> Option<ValueKind> {
        match value_kind_of(ty) {
            Some(kind) => Some(kind),
            None => {
                self.error(
                    ty.span(),
                    format!(
                        "record field type '{}' is out of scope for this step's minimal \
                         codegen (only INTEGER/BOOLEAN/STRING[n] fields are supported)",
                        describe_type(ty)
                    ),
                );
                None
            }
        }
    }

    /// `VAR`宣言の型が（`TYPE`宣言経由・無名のいずれかで）レコード型かどうかを
    /// 判定する（Step 20）。
    fn resolve_record_type_expr(&mut self, ty: &TypeExpr) -> RecordResolution {
        match ty {
            TypeExpr::Record { fields, span, .. } => {
                match self.build_record_layout(fields, *span) {
                    Some(layout) => {
                        let id = RecordId(self.record_layouts.len());
                        self.record_layouts.push(layout);
                        RecordResolution::Valid(id)
                    }
                    None => RecordResolution::Invalid,
                }
            }
            TypeExpr::Named(ident) => match self.record_type_names.get(&normalize(&ident.name)) {
                Some(id) => RecordResolution::Valid(*id),
                None => RecordResolution::NotRecord,
            },
            _ => RecordResolution::NotRecord,
        }
    }

    /// レコード型の`VAR`宣言（グローバルのみ。Step 20）。
    /// [`crate::codegen`]モジュールドキュメントの「スコープ」参照:
    /// `STRING[n]`/配列と同じ前例に倣い、`PROGRAM`直下のグローバル変数の
    /// みサポートする（`PROCEDURE`/`FUNCTION`のローカル変数・仮引数の位置に
    /// 現れたレコード型は、この関数を経由せず従来通り[`Self::build_locals`]/
    /// [`Self::build_params`]が使う[`value_kind_of`]が`None`を返すことで、
    /// 自然に「スコープ外」の診断へ流れる）。
    fn declare_record_vars(&mut self, decl: &VarDecl, id: RecordId) {
        let total_words = self.record_layouts[id.0].total_words;
        let kind = ValueKind::Record(RecordKind { id, total_words });
        for name in &decl.names {
            let address = self.alloc_words(total_words);
            self.vars
                .insert(normalize(&name.name), VarSlot { address, kind });
        }
    }

    /// `TypeExpr::Array`をこのクレートのスコープが対応する[`ArrayKind`]へ
    /// 解決する（Step 19、Step 21から[`Self::build_params`]でも再利用）。
    /// [`ArrayKind`]のドキュメント「スコープ」参照: 1次元・`INTEGER`/
    /// `BOOLEAN`要素のみ。それ以外（多次元・その他の要素型・配列でない型）
    /// は診断のみ積んで`None`を返す（[`Self::declare_vars`]の既存の型と
    /// 同じ「診断1件でも`generate`全体が`Err`になる」方針に乗るため、以降の
    /// アドレス割り当てのズレは問題にならない）。`ty`が`TypeExpr::Array`
    /// ではない場合は、診断を出さず静かに`None`を返す（呼び出し元が他の
    /// 型として解決を続けられるようにするため。[`Self::resolve_record_type_expr`]
    /// と同じ設計）。
    fn resolve_array_type_expr(&mut self, ty: &TypeExpr) -> Option<ArrayKind> {
        let TypeExpr::Array {
            index_type,
            element_type,
            span,
            ..
        } = ty
        else {
            return None;
        };

        let element = match element_type.as_ref() {
            TypeExpr::Integer(_) => ScalarValueKind::Int,
            TypeExpr::Boolean(_) => ScalarValueKind::Bool,
            other => {
                self.error(
                    other.span(),
                    format!(
                        "array element type '{}' is out of scope for this step's minimal \
                         codegen (only INTEGER/BOOLEAN array elements are supported; in \
                         particular, multi-dimensional arrays are out of scope)",
                        describe_type(other)
                    ),
                );
                return None;
            }
        };

        let (low, high) = match index_type.as_ref() {
            TypeExpr::Subrange {
                low: Literal::Int(lo, _),
                high: Literal::Int(hi, _),
                ..
            } => (*lo, *hi),
            other => {
                self.error(
                    other.span(),
                    "this array index range form is out of scope for this step's minimal \
                     codegen (only an INTEGER literal subrange 'low..high' is supported)",
                );
                return None;
            }
        };

        let arr = ArrayKind { low, high, element };
        // `low <= high`は`wasd-sema`が既に保証済み（`ArrayKind`のドキュメント
        // 参照）なので`element_count()`は必ず正だが、要素数自体が16bitワード
        // 数に収まるとは限らない（例: `ARRAY [1..100000] OF INTEGER`）ため、
        // ここで明示的に診断する。
        if u16::try_from(arr.element_count()).is_err() {
            self.error(
                *span,
                format!(
                    "array has too many elements for this step's minimal codegen ({} elements \
                     does not fit in a 16-bit word count)",
                    arr.element_count()
                ),
            );
            return None;
        }
        Some(arr)
    }

    /// `ARRAY [low..high] OF element`のグローバル`VAR`宣言（Step 19）。
    fn declare_array_vars(&mut self, decl: &VarDecl) {
        let Some(arr) = self.resolve_array_type_expr(&decl.ty) else {
            return;
        };
        let kind = ValueKind::Array(arr);
        let words = word_size_of(kind);

        for name in &decl.names {
            let address = self.alloc_words(words);
            self.vars
                .insert(normalize(&name.name), VarSlot { address, kind });
        }
    }

    /// グローバルデータ領域から`words`ワード分を確保し、先頭アドレスを
    /// 返す（`STRING[n]`のような複数ワードを占める変数向け。
    /// [`Self::alloc_slot`]は`words = 1`の場合の薄いラッパー）。
    fn alloc_words(&mut self, words: u16) -> Address {
        let address = Address(self.next_address);
        self.next_address += words;
        address
    }

    fn alloc_slot(&mut self) -> Address {
        self.alloc_words(1)
    }

    // ---- PROCEDURE/FUNCTION宣言の登録 ----

    /// 仮引数の型を検査し、活性化レコード内のオフセットを割り当てる。
    /// `param_base`は仮引数領域の先頭オフセット（`5 + data_size`）。
    /// 型がサポート外の仮引数は診断のみ積んでスロットを割り当てない
    /// （[`Self::declare_vars`]と同じ方針。この宣言全体はどのみち
    /// `Err`を返すことになるため、以降のオフセットのズレは問題にならない）。
    ///
    /// `STRING[n]`の仮引数（Step 18から。値仮引数・`VAR`仮引数のいずれも）は
    /// スロットにアドレスを格納する（`indirect = true`）ため、
    /// 活性化レコード内では他の仮引数と同じ1ワードで済む
    /// （`FrameSlot`/`Self::gen_string_value_arg`のドキュメント参照）。
    /// `STRING[n]`のローカル変数（`PROCEDURE`/`FUNCTION`本体内の`VAR`宣言）は
    /// 依然としてスコープ外（[`Self::build_locals`]参照）だが、仮引数は
    /// データそのものではなくアドレスしか持たないため、この制約に抵触しない。
    ///
    /// # UNCONFIRMED/TODO: 配列・レコード型の仮引数はStep 21から許可（仮実装）
    ///
    /// Step 21のタスク依頼に従い、配列・レコード型を仮引数の型として許可
    /// する。ただし本来のPascalの値渡し意味論（呼び出し側でコピーを作成し、
    /// そのコピーのアドレスを渡す）は実装せず、`STRING[n]`の値仮引数
    /// （[`Self::gen_string_value_arg`]、コピーを作る）とは異なり、**コピーを
    /// 作らず元の変数のアドレスをそのまま渡す**仮実装とする（意図的な
    /// 単純化。タスク依頼の明示的な指示）。そのため`by_ref`（`VAR`）で
    /// あるかどうかに関わらず、配列・レコード型の仮引数は常に
    /// `indirect = true`（スロットにアドレスを格納）になる。結果として、
    /// 呼び出された側での配列要素・レコードフィールドへの変更が呼び出し元
    /// にも反映されてしまう（実質的に参照渡しのように振る舞う）という
    /// 既知の制限がある。正しい値渡し意味論（コピー生成）は、将来の`VAR`
    /// パラメータの正式な構文・意味論の実装と合わせて別ステップで対応する
    /// 予定（呼び出し側のコード生成は[`Self::gen_array_or_record_value_arg`]、
    /// リポジトリの`README.md`「既知の制限」も参照）。
    fn build_params(&mut self, params: &[ParamDecl], param_base: u16) -> Vec<(String, FrameSlot)> {
        let mut result = Vec::new();
        for p in params {
            // `matches!`で先に`TypeExpr::Array`かどうかを判定してから
            // `resolve_array_type_expr`を呼ぶ（`Self::declare_vars`と同じ
            // パターン）: 不正な配列型（多次元・非対応要素型など）は
            // `resolve_array_type_expr`が既に診断を出して`None`を返すため、
            // それをさらにレコード・スカラーとして解決しようとして
            // 二重に診断を出してしまうのを避ける。
            let kind = if matches!(p.ty, TypeExpr::Array { .. }) {
                match self.resolve_array_type_expr(&p.ty) {
                    Some(arr) => Some(ValueKind::Array(arr)),
                    None => continue,
                }
            } else {
                match self.resolve_record_type_expr(&p.ty) {
                    RecordResolution::Valid(id) => {
                        let total_words = self.record_layouts[id.0].total_words;
                        Some(ValueKind::Record(RecordKind { id, total_words }))
                    }
                    // フィールド型・サイズのエラーは`resolve_record_type_expr`が
                    // 既に報告済みなので、ここでは追加の診断を出さずスキップする。
                    RecordResolution::Invalid => continue,
                    RecordResolution::NotRecord => value_kind_of(&p.ty),
                }
            };
            let Some(kind) = kind else {
                self.error(
                    p.ty.span(),
                    format!(
                        "parameter type '{}' is out of scope for this step's minimal codegen \
                         (only INTEGER/BOOLEAN/STRING[n]/ARRAY/RECORD are supported)",
                        describe_type(&p.ty)
                    ),
                );
                continue;
            };
            let indirect = p.by_ref
                || matches!(
                    kind,
                    ValueKind::StringN(_) | ValueKind::Array(_) | ValueKind::Record(_)
                );
            let slot = FrameSlot {
                address: Address(param_base + result.len() as u16),
                by_ref: p.by_ref,
                indirect,
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
            // Step 16: `build_params`と同じ理由で、`STRING[n]`のローカル
            // 変数（`PROCEDURE`/`FUNCTION`本体内の`VAR`宣言）はまだ
            // サポートしない。
            if matches!(kind, ValueKind::StringN(_)) {
                self.error(
                    decl.ty.span(),
                    "STRING[n] local variables are out of scope for this step's minimal \
                     codegen (only PROGRAM-level global STRING[n] variables are supported)",
                );
                continue;
            }
            for name in &decl.names {
                let slot = FrameSlot {
                    address: Address(5 + count),
                    by_ref: false,
                    indirect: false,
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
                    indirect: slot.indirect,
                    offset: 0,
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
                indirect: false,
                offset: 0,
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
                indirect: false,
                offset: 0,
            }
        } else {
            let address = self.alloc_slot();
            ResolvedVar {
                level: Level(0),
                address,
                indirect: false,
                offset: 0,
            }
        }
    }

    /// `indirect`な`ResolvedVar`について、スロットから読んだアドレスへ
    /// `offset`（コンパイル時定数、実行時に加算）を足し込む命令列を発行する
    /// （`offset == 0`なら何も発行しない）。[`ResolvedVar::offset`]の
    /// ドキュメント参照。呼び出し元は、スタック上に既にスロットの中身
    /// （アドレス）が積まれている状態でこれを呼ぶ。
    fn emit_indirect_offset(&mut self, offset: u16, span: Span) {
        if offset != 0 {
            self.emit_ldc_int(offset as i64, span);
            self.emit(UnconfirmedOp::Adi.into(), span);
        }
    }

    /// 解決済みの変数を読み込み、スタックへ値を積む。`VAR`仮引数
    /// （`by_ref`）の場合は、スロットに格納されているアドレスをまず
    /// [`UnconfirmedOp::Lod`]で読み、（[`ResolvedVar::offset`]が非0なら
    /// それを加算した上で）続けて[`UnconfirmedOp::Ind`]で参照先の値を
    /// デリファレンスする。
    fn gen_load_resolved(&mut self, resolved: ResolvedVar, span: Span) {
        self.emit(
            UnconfirmedOp::Lod(resolved.level, resolved.address).into(),
            span,
        );
        if resolved.indirect {
            self.emit_indirect_offset(resolved.offset, span);
            self.emit(UnconfirmedOp::Ind.into(), span);
        }
    }

    /// 解決済みの変数へ値を格納する。`gen_value`が値を生成するコードを
    /// 発行するコールバックで、`by_ref`の場合は先にスロットからアドレスを
    /// 読み出し（[`ResolvedVar::offset`]が非0ならそれを加算し）てから値を
    /// 積み、[`UnconfirmedOp::Sti`]で間接ストアする（アドレスが先、値が後
    /// というスタック順を仮定している。[`UnconfirmedOp::Sti`]のドキュメント
    /// 参照）。
    fn gen_store_resolved(
        &mut self,
        resolved: ResolvedVar,
        span: Span,
        gen_value: impl FnOnce(&mut Self),
    ) {
        if resolved.indirect {
            self.emit(
                UnconfirmedOp::Lod(resolved.level, resolved.address).into(),
                span,
            );
            self.emit_indirect_offset(resolved.offset, span);
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
    /// [`ResolvedVar::offset`]が非0（間接参照経由のレコードフィールド）な
    /// 場合はその分を加算する。
    fn gen_address_of_resolved(&mut self, resolved: ResolvedVar, span: Span) {
        if resolved.indirect {
            self.emit(
                UnconfirmedOp::Lod(resolved.level, resolved.address).into(),
                span,
            );
            self.emit_indirect_offset(resolved.offset, span);
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
        let ident = match target {
            Expr::Identifier(ident) => ident,
            // Step 19: 配列要素への代入（`arr[i] := value`）。
            Expr::IndexAccess { array, index, .. } => {
                self.gen_array_element_address(array, index, target.span());
                self.gen_expr(value);
                self.emit(UnconfirmedOp::Sti.into(), span);
                return;
            }
            // Step 20: レコードフィールドへの代入（`rec.field := value`）。
            // フィールドのアドレスは常にコンパイル時に確定するので、配列
            // 要素（間接アドレッシング、`Sti`経由）とは異なり、
            // `Self::gen_store_resolved`（直接記憶方式向け、`Str`のみ）へ
            // そのまま流せる（`Self::resolve_field_access`のドキュメント
            // 「設計判断」参照）。`STRING[n]`フィールドへの文字列リテラル
            // 代入のみ、通常の変数と同じく専用のヘルパーを経由する。
            Expr::FieldAccess { record, field, .. } => {
                match self.resolve_field_access(record, field) {
                    Some(resolved) => match self.field_kind(record, field) {
                        Some(ValueKind::StringN(max_len)) => {
                            self.gen_string_literal_assignment(resolved, max_len, value, span);
                        }
                        _ => {
                            self.gen_store_resolved(resolved, span, |g| g.gen_expr(value));
                        }
                    },
                    None => {
                        self.gen_expr(value);
                    }
                }
                return;
            }
            _ => {
                self.error(
                    target.span(),
                    "assignment targets other than a simple variable, array element, or record \
                     field (pointer lvalues) are out of scope for this step's minimal codegen",
                );
                return;
            }
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
            Some(resolved) => match self.lookup_kind(&key) {
                Some(ValueKind::StringN(max_len)) => {
                    self.gen_string_literal_assignment(resolved, max_len, value, span);
                }
                // Step 19: 配列全体を1ワードのスカラーとして代入することは
                // できない（`arr2 := arr1;`のような、両辺が同じ配列型の
                // 「配列全体の代入」は伝統的なPascalの意味論としては妥当で、
                // `wasd-sema`の`assignment_compatible`も構造的に同じ配列型
                // 同士を許してしまう。これをここでガードせず
                // `gen_store_resolved`にそのまま流すと、配列の先頭1ワードだけ
                // を読み書きする誤ったコードを黙って生成してしまう
                // （[`Self::gen_identifier_load`]の同種のガードも参照）。
                Some(ValueKind::Array(_)) => {
                    self.error(
                        ident.span,
                        "whole-array assignment ('a := b' where both sides are arrays) is out \
                         of scope for this step's minimal codegen (only assigning to a single \
                         indexed element, 'a[i] := value', is supported)",
                    );
                    self.gen_expr(value);
                }
                // Step 20: 配列と同じ理由で、レコード全体の代入
                // (`rec2 := rec1;`)もガードする。
                Some(ValueKind::Record(_)) => {
                    self.error(
                        ident.span,
                        "whole-record assignment ('a := b' where both sides are records) is \
                         out of scope for this step's minimal codegen (only assigning to a \
                         single field, 'a.field := value', is supported)",
                    );
                    self.gen_expr(value);
                }
                _ => {
                    self.gen_store_resolved(resolved, span, |g| g.gen_expr(value));
                }
            },
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

    /// 配列要素`array[index]`の**アドレス**（[`UnconfirmedOp::Lda`]と同じ、
    /// `stack`中の絶対インデックスを表す1ワード）を計算してスタックへ積む
    /// （Step 19）。
    ///
    /// # 設計判断: 専用オペコード（`IXA`相当）を新設しない
    ///
    /// 実機のUCSD p-codeには配列添字のアドレス計算専用の命令
    /// （伝聞では`IXA`）があるとされるが、このセッションでは一次資料に
    /// 一切あたれておらず、命令の存在・名称・オペコード番号のいずれも
    /// 確認できない（[`crate::opcode::UnconfirmedOp`]のドキュメント
    /// 「一次資料で確認できていない」の節と同じ状況）。
    ///
    /// 一方、本IRの`STRING[n]`向けに既に導入済みの[`UnconfirmedOp::Lda`]
    /// （アドレスを1ワードの整数値としてスタックへ積む）という設計を
    /// 前提にすると、配列要素のアドレスは既存の整数演算命令だけで
    /// 合成できる: 配列の先頭要素（`low`番目）のアドレスを`LDA`で積み、
    /// 添字の値を評価し、`LDC <low>`で下限を積んで`SBI`で引き算すれば
    /// 「先頭からのオフセット（0起点）」が得られ、それを`ADI`で先頭
    /// アドレスへ加算すれば目的の要素のアドレスになる（要素は本ステップの
    /// スコープでは常に1ワードなので、オフセットへの乗算は不要）。
    /// この計算はすべて`ADI`/`SBI`という既存の（かつオペコード番号自体は
    /// 未確認だが、少なくとも算術命令として確立済みの）命令の組み合わせに
    /// 過ぎず、新たな「発明」を追加しない。そのため`pmachine-core`側も
    /// 無改修でこの配列インデックスを実行できる
    /// （`crates/pmachine-core`にオペコード追加が不要な理由）。
    ///
    /// 計算結果のアドレスは、値を読むなら[`UnconfirmedOp::Ind`]、書くなら
    /// 値を積んでから[`UnconfirmedOp::Sti`]と組み合わせて使う（`VAR`仮引数
    /// の間接アドレッシングと全く同じ後続命令。[`Self::gen_load_resolved`]/
    /// [`Self::gen_store_resolved`]のドキュメント参照）。
    ///
    /// # スコープ: 単純な配列変数のみ
    ///
    /// `array`は単純な識別子（`PROGRAM`直下のグローバル配列変数）のみ
    /// サポートする。多次元配列（`array`自体が`IndexAccess`になっている
    /// ケース）・レコードのフィールドである配列・`VAR`仮引数として渡された
    /// 配列はいずれも今回のスコープ外（[`ArrayKind`]のドキュメント参照）。
    ///
    /// # 範囲チェックなし
    ///
    /// `wasd-sema`のコンパイル時定数添字に対する範囲チェック
    /// （`crates/wasd-sema/src/typeck.rs`の`infer_index_access_type`
    /// ドキュメント「実行時の範囲チェックは今回のscopeでは行わない」）と
    /// 歩調を合わせ、本クレートも実行時の範囲チェック命令（実機の`CHK`
    /// 相当）は発行しない。範囲外アクセスは`pmachine-core`側の
    /// `AddressOutOfRange`（配列がグローバルデータ領域の末尾付近にある
    /// 場合）で検出されることもあれば、他の変数の領域を静かに読み書き
    /// してしまうこともあり得る、という制約が残る。
    fn gen_array_element_address(&mut self, array_expr: &Expr, index_expr: &Expr, span: Span) {
        let Expr::Identifier(ident) = array_expr else {
            self.error(
                array_expr.span(),
                "array indexing is only supported on a simple array variable (not a nested \
                 index, field access, or other expression) by this step's minimal codegen",
            );
            self.emit(UnconfirmedOp::Ldc(0).into(), span);
            return;
        };
        let key = normalize(&ident.name);
        let Some(ValueKind::Array(arr)) = self.lookup_kind(&key) else {
            self.error(
                ident.span,
                format!(
                    "'{}' is not a known array variable in this scope",
                    ident.name
                ),
            );
            self.emit(UnconfirmedOp::Ldc(0).into(), span);
            return;
        };
        let Some(resolved) = self.resolve_var(&key) else {
            unreachable!("lookup_kind and resolve_var share the same name resolution order");
        };

        self.gen_address_of_resolved(resolved, span);
        self.gen_expr(index_expr);
        self.emit_ldc_int(arr.low, index_expr.span());
        self.emit(UnconfirmedOp::Sbi.into(), span);
        self.emit(UnconfirmedOp::Adi.into(), span);
    }

    /// レコードフィールドアクセス`record.field`を[`ResolvedVar`]として解決
    /// する（Step 20）。読み込み（[`Self::gen_expr`]の`Expr::FieldAccess`腕）・
    /// 書き込み（[`Self::gen_assignment`]の同腕）・`WriteLn(rec.field)`
    /// （[`Self::gen_writeln_string_var`]）のいずれもこの関数を経由する。
    ///
    /// # 設計判断: 専用の間接アドレス計算命令を経由しない
    ///
    /// Step 19の配列添字（[`Self::gen_array_element_address`]）は添字が
    /// 実行時に決まる値なので、`LDA`で先頭アドレスを積んでから`ADI`/`SBI`
    /// で実行時にオフセットを加算する、という間接アドレッシングの命令列
    /// （`IXA`相当の合成）が必要だった。一方、レコードのフィールド
    /// （`rec.field`の`field`）は識別子であり式ではないので、そのオフセット
    /// は常に**コンパイル時定数**である（[`RecordLayout`]のドキュメント
    /// 参照）。
    ///
    /// このIRの[`UnconfirmedOp::Lod`]/[`UnconfirmedOp::Str`]は、そもそも
    /// 「レベル差 + 絶対ワードアドレス」を直接オペランドに取れる設計に
    /// なっている（グローバル変数への直接アクセスに使われているのと同じ
    /// 命令。タスク依頼が示唆する「LDO等、ベース+定数オフセットを直接
    /// 扱える命令」に相当）。そのため、レコード変数のベースアドレス
    /// （[`Self::resolve_var`]が返す`ResolvedVar::address`。グローバル変数
    /// なら`Self::vars`のアドレス、ローカル変数なら活性化レコード内の
    /// オフセット）へフィールドのオフセットをコンパイル時に加算した結果を
    /// そのまま新しい`ResolvedVar`として返すだけで完結し、Step 19の配列の
    /// ような`Lda`+`Ind`/`Sti`の間接アドレッシングの命令列を経由する必要が
    /// ない（呼び出し元は返された`ResolvedVar`を[`Self::gen_load_resolved`]/
    /// [`Self::gen_store_resolved`]へそのまま渡すだけでよく、これらは
    /// `indirect = false`の場合`Lod`/`Str`を1つ発行するだけなので、通常の
    /// スカラー変数と全く同じコード量で済む）。
    ///
    /// UNCONFIRMED: 一次資料（SofTech Microsystems, *UCSD p-System and
    /// UCSD Pascal Version IV: Internal Architecture Guide*）にレコード型の
    /// メモリレイアウト・フィールドアクセス専用命令の記述は見当たらな
    /// かった（`docs/research/ucsd-pascal-primary-sources.md`参照。この
    /// セッションでも改めて`archive.org`への`WebFetch`を試みたが
    /// `EGRESS_BLOCKED`だった）。上記の判断は一般的なPascal実装の理解
    /// （フィールドオフセットはコンパイル時定数であり、ベースアドレスへの
    /// 加算だけで済む）に基づく。
    ///
    /// # スコープ: 単純な識別子のみ（Step 21から`VAR`/値仮引数のレコードも含む）
    ///
    /// `record`は単純な識別子のみサポートする。ネストしたフィールド
    /// アクセス（レコード内レコード）・配列要素のフィールドはいずれも
    /// 今回のスコープ外（[`crate::codegen`]モジュールドキュメント参照）。
    /// 診断は呼び出し元ではなくこの関数自身が発行し、`None`を返す
    /// （[`Self::gen_array_element_address`]と異なりダミー値は積まない。
    /// 呼び出し元がロード/ストアいずれの文脈かによって適切なダミー処理
    /// （`Ldc(0)`を積む、あるいは値だけ評価してストアは諦める）が異なる
    /// ため、ダミー処理の選択は呼び出し元に委ねる）。
    ///
    /// # Step 21: `VAR`/値仮引数として渡されたレコードのフィールドアクセス
    ///
    /// `record`がレコード型の`VAR`仮引数、または（本ステップの仮実装方針に
    /// 基づく）値仮引数である場合、`base`（[`Self::resolve_var`]の結果）は
    /// `indirect = true`になる。この場合、フィールドのオフセットは依然
    /// コンパイル時定数だが、それを足し込む先はスロットの**中身**
    /// （実行時にしか分からないレコード本体のアドレス）であって、スロット
    /// 自身のアドレスではない。そのため`base.indirect`が真の場合は
    /// オフセットを`ResolvedVar::address`へ静的に加算せず、代わりに
    /// [`ResolvedVar::offset`]として持たせ、実際の加算を
    /// [`Self::gen_load_resolved`]/[`Self::gen_store_resolved`]/
    /// [`Self::gen_address_of_resolved`]が実行時に（`Lod`でスロットから
    /// アドレスを読んだ後、必要なら`Ldc`+`Adi`で加算してから`Ind`/`Sti`）
    /// 行う（配列添字アクセス[`Self::gen_array_element_address`]と同種の
    /// 間接アドレッシングだが、添字がコンパイル時定数である分、条件付きで
    /// `Ldc`+`Adi`を省略できる点が異なる）。
    fn resolve_field_access(
        &mut self,
        record_expr: &Expr,
        field: &Identifier,
    ) -> Option<ResolvedVar> {
        let Expr::Identifier(ident) = record_expr else {
            self.error(
                record_expr.span(),
                "record field access is only supported on a simple record variable (not a \
                 nested field access, array element, or other expression) by this step's \
                 minimal codegen",
            );
            return None;
        };
        let key = normalize(&ident.name);
        let Some(ValueKind::Record(rec_kind)) = self.lookup_kind(&key) else {
            self.error(
                ident.span,
                format!(
                    "'{}' is not a known record variable in this scope",
                    ident.name
                ),
            );
            return None;
        };
        let Some(base) = self.resolve_var(&key) else {
            unreachable!("lookup_kind and resolve_var share the same name resolution order");
        };

        let field_key = normalize(&field.name);
        let field_offset = self.record_layouts[rec_kind.id.0]
            .fields
            .iter()
            .find(|f| f.name == field_key)
            .map(|f| f.offset);
        let Some(offset) = field_offset else {
            self.error(
                field.span,
                format!("record type has no field '{}'", field.name),
            );
            return None;
        };

        if base.indirect {
            // `base`自体が`VAR`/値仮引数（スロットにレコード本体のアドレスを
            // 格納している）である場合。`resolve_field_access`のドキュメント
            // 「Step 21」参照: オフセットの加算は実行時に委ねる。
            Some(ResolvedVar {
                level: base.level,
                address: base.address,
                indirect: true,
                offset: base.offset + offset,
            })
        } else {
            // 直接記憶方式（`PROGRAM`直下のグローバルレコード変数）。
            // オフセットをコンパイル時に`address`へ静的に加算できる
            // （このモジュールドキュメントの「設計判断」参照）。
            Some(ResolvedVar {
                level: base.level,
                address: Address(base.address.0 + offset),
                indirect: false,
                offset: 0,
            })
        }
    }

    /// フィールドの種類（[`ValueKind`]）を、診断を出さずに解決する
    /// （Step 20）。[`Self::infer_expr_kind`]の`Expr::FieldAccess`腕、および
    /// [`Self::gen_assignment`]がSTRING[n]フィールドかどうかを判定する際に
    /// 使う。
    fn field_kind(&self, record_expr: &Expr, field: &Identifier) -> Option<ValueKind> {
        let rec_kind = match self.infer_expr_kind(record_expr)? {
            ValueKind::Record(rec) => rec,
            _ => return None,
        };
        let field_key = normalize(&field.name);
        self.record_layouts[rec_kind.id.0]
            .fields
            .iter()
            .find(|f| f.name == field_key)
            .map(|f| f.kind)
    }

    /// `s := 'literal';`（`s`が`STRING[max_len]`）のコード生成。
    ///
    /// # スコープ: 文字列リテラルの代入のみ
    ///
    /// タスクのスコープ（このステップでは文字列演算・変数間代入は対象外。
    /// `crate`モジュールドキュメント参照）に従い、右辺が文字列リテラル
    /// でない場合（他の`STRING[n]`変数からの代入等）はスコープ外として
    /// エラー報告のみ行う。
    ///
    /// # 生成する命令列: 長さワード + 文字ごとに1ワード
    ///
    /// [`crate::builtin::BUILTIN_WRITELN_STRVAR`]のドキュメント「メモリ
    /// レイアウト」で説明した単純化されたレイアウト（1ワード=1文字）に
    /// 従い、まず長さをアドレス（`resolved.address`）へ`STR`し、続けて
    /// 各文字コードを`resolved.address + 1 + i`へ`STR`する。新規オペコードは
    /// 一切必要とせず、既存の`LDC`/`STR`のみで表現できる。
    fn gen_string_literal_assignment(
        &mut self,
        resolved: ResolvedVar,
        max_len: u8,
        value: &Expr,
        span: Span,
    ) {
        if resolved.indirect {
            self.error(
                span,
                "assigning to a STRING[n] parameter (VAR or value) is out of scope for this \
                 step's minimal codegen",
            );
            return;
        }
        let Expr::StringLiteral(text, lit_span) = value else {
            self.error(
                value.span(),
                "assigning a value other than a string literal to a STRING[n] variable is out \
                 of scope for this step's minimal codegen",
            );
            return;
        };

        self.emit_string_literal_words(
            resolved.level,
            resolved.address,
            max_len,
            text,
            *lit_span,
            span,
        );
    }

    /// 文字列リテラルの内容を、宛先（`dest_level`/`dest`が指す先頭ワード）へ
    /// 書き込む命令列を発行する（「長さ1ワード＋文字ごとに1ワード」という
    /// 単純化されたレイアウト。[`crate::builtin::BUILTIN_WRITELN_STRVAR`]の
    /// ドキュメント「メモリレイアウト」参照）。[`Self::gen_string_literal_assignment`]
    /// （`s := 'literal';`）と[`Self::gen_string_value_arg`]（`STRING[n]`値
    /// 引数として渡す文字列リテラルを一時領域へ書き込む、Step 18）の両方が
    /// この共通実装を使う。宣言長を超える場合は診断のみ積んで`false`を返す。
    fn emit_string_literal_words(
        &mut self,
        dest_level: Level,
        dest: Address,
        max_len: u8,
        text: &str,
        lit_span: Span,
        span: Span,
    ) -> bool {
        let chars: Vec<char> = text.chars().collect();
        if chars.len() > max_len as usize {
            self.error(
                lit_span,
                format!(
                    "string literal of length {} does not fit in STRING[{max_len}]",
                    chars.len()
                ),
            );
            return false;
        }

        self.emit(UnconfirmedOp::Ldc(chars.len() as i16).into(), lit_span);
        self.emit(UnconfirmedOp::Str(dest_level, dest).into(), span);
        for (i, ch) in chars.iter().enumerate() {
            // UNCONFIRMED: ASCII範囲（0..=127）のみ想定。それを超える文字
            // コードのp-machine上での扱いは一次資料未確認（このクレートの
            // 他の箇所と同様、本ステップのスコープ外）。
            self.emit(UnconfirmedOp::Ldc(*ch as i16).into(), lit_span);
            let char_address = Address(dest.0 + 1 + i as u16);
            self.emit(UnconfirmedOp::Str(dest_level, char_address).into(), span);
        }
        true
    }

    /// 直接記憶方式（`indirect`でない）の`STRING[n]`変数の中身を、宛先
    /// （`dest_level`/`dest`が指す先頭ワード）へ1ワードずつコピーする
    /// 命令列を発行する（[`Self::gen_string_value_arg`]が、`STRING[n]`変数を
    /// そのまま別の`STRING[n]`値引数として渡す場合に使う。Step 18）。
    fn emit_string_copy_words(
        &mut self,
        dest_level: Level,
        dest: Address,
        src: ResolvedVar,
        max_len: u8,
        span: Span,
    ) {
        debug_assert!(
            !src.indirect,
            "emit_string_copy_words requires a directly-stored STRING[n] source"
        );
        for i in 0..=(max_len as u16) {
            self.emit(
                UnconfirmedOp::Lod(src.level, Address(src.address.0 + i)).into(),
                span,
            );
            self.emit(
                UnconfirmedOp::Str(dest_level, Address(dest.0 + i)).into(),
                span,
            );
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
    /// - 引数1つ（文字列リテラル以外の`WriteLn(expr)`）で`STRING[n]`変数の
    ///   場合（Step 16）: `expr`を評価する（`gen_expr`で値をスタックへ積む）
    ///   のではなく、変数自身の**アドレス**を積んで`BUILTIN_WRITELN_STRVAR`
    ///   を呼ぶ（[`Self::gen_writeln_string_var`]、
    ///   [`crate::builtin::BUILTIN_WRITELN_STRVAR`]のドキュメント参照）。
    /// - 引数1つ（それ以外の`WriteLn(expr)`）: `expr`を評価してスタックへ
    ///   積み、その式の種類（[`Self::infer_expr_kind`]）に応じて
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
            [arg] if matches!(self.infer_expr_kind(arg), Some(ValueKind::StringN(_))) => {
                self.gen_writeln_string_var(arg, span);
            }
            [arg] => {
                let kind = self.infer_expr_kind(arg);
                self.gen_expr(arg);
                let proc = match kind {
                    Some(ValueKind::Int) => BUILTIN_WRITELN_INT,
                    Some(ValueKind::Bool) => BUILTIN_WRITELN_BOOL,
                    Some(ValueKind::StringN(_)) => {
                        unreachable!("STRING[n] arguments are handled by the previous match arm")
                    }
                    // Step 19/20: `WriteLn(arr)`/`WriteLn(rec)`（配列・
                    // レコード全体）。`self.gen_expr(arg)`（上）が既に
                    // `Self::gen_identifier_load`のガードで診断を出し、
                    // ダミー値を積んでいるため、ここでは追加の診断を出さず
                    // ダミーの`proc`番号を選ぶだけでよい。
                    Some(ValueKind::Array(_)) | Some(ValueKind::Record(_)) => BUILTIN_WRITELN_INT,
                    None => {
                        self.error(
                            arg.span(),
                            "WriteLn only supports INTEGER/BOOLEAN/STRING[n]/string literal \
                             arguments in this step's minimal codegen",
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

    /// `WriteLn(s)`（`s`が`STRING[n]`変数）のコード生成（Step 16）。
    ///
    /// `s`自体の値ではなく**アドレス**（[`Self::gen_address_of_resolved`]）
    /// をスタックへ積んでから`BUILTIN_WRITELN_STRVAR`を呼ぶ。呼び出された
    /// 側（`pmachine-core`）がそのアドレスから長さ＋文字データを読み出す
    /// （[`crate::builtin::BUILTIN_WRITELN_STRVAR`]のドキュメント参照）。
    ///
    /// スコープ: 単純な変数参照のみ（[`Self::gen_var_arg`]と同じ制限）。
    ///
    /// `s`が`STRING[n]`仮引数（値仮引数・`VAR`仮引数のいずれも。Step 18から
    /// サポート）である場合、そのスロットに格納されているのは既に文字列
    /// データ自身の**アドレス**なので、[`Self::gen_address_of_resolved`]が
    /// （`indirect`を見て）`LOD`だけを発行し、正しくそのアドレスを取り出す
    /// （`s`が直接記憶方式のグローバル`STRING[n]`変数であれば、代わりに
    /// `LDA`でアドレスを計算する）。いずれの場合も呼び出し側で`indirect`を
    /// 区別する必要はない。
    ///
    /// Step 20から、単純な変数参照に加えて`STRING[n]`フィールドへの
    /// レコードフィールドアクセス（`WriteLn(rec.field)`）もサポートする
    /// （[`Self::resolve_field_access`]参照。フィールドのアドレスも
    /// コンパイル時に確定するため、単純な変数と同じ`ResolvedVar`の
    /// 仕組みにそのまま乗る）。
    fn gen_writeln_string_var(&mut self, arg: &Expr, span: Span) {
        let resolved = match arg {
            Expr::Identifier(ident) => {
                let key = normalize(&ident.name);
                match self.resolve_var(&key) {
                    Some(resolved) => Some(resolved),
                    None => {
                        self.error(
                            ident.span,
                            format!("'{}' is not a known variable in this scope", ident.name),
                        );
                        None
                    }
                }
            }
            Expr::FieldAccess { record, field, .. } => self.resolve_field_access(record, field),
            _ => {
                self.error(
                    arg.span(),
                    "WriteLn(STRING[n]) only supports a simple variable reference or a record \
                     field access in this step's minimal codegen",
                );
                None
            }
        };
        if let Some(resolved) = resolved {
            self.gen_address_of_resolved(resolved, span);
            self.emit(
                ConfirmedOp::Cxg(KERNEL_SEGMENT, BUILTIN_WRITELN_STRVAR).into(),
                span,
            );
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
            // Step 19: `arr[i]`の種類は、配列の要素の種類（`INTEGER`/
            // `BOOLEAN`のみ。`ArrayKind`のドキュメント参照）そのもの。
            Expr::IndexAccess { array, .. } => match self.infer_expr_kind(array) {
                Some(ValueKind::Array(arr)) => Some(arr.element.into()),
                _ => None,
            },
            // Step 20: `rec.field`の種類は、フィールドの種類そのもの
            // （[`Self::field_kind`]参照）。
            Expr::FieldAccess { record, field, .. } => self.field_kind(record, field),
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
    /// サポート）、`STRING[n]`の値仮引数には呼び出し元が新規確保した
    /// 一時領域のアドレスを（[`Self::gen_string_value_arg`]、Step 18）、
    /// 配列・レコードの値仮引数には元の変数のアドレスをそのまま
    /// （[`Self::gen_array_or_record_value_arg`]、Step 21の仮実装。コピーは
    /// 作らない）、それ以外の値仮引数には値を（[`Self::gen_expr`]）積む。
    ///
    /// # 引数の評価順序: 仮引数の並び順（左から右）
    ///
    /// 引数は`args.iter().zip(params.iter())`が辿る順、すなわち呼び出し式に
    /// 書かれた実引数の並び順（＝仮引数の宣言順）で左から右へ評価・発行する
    /// （一次資料由来のCONFIRMED済みの呼び出し規約。`crate`モジュール
    /// ドキュメント「呼び出し規約とRPUのBパラメータ」参照）。この順序で
    /// 積んだ結果が、そのまま活性化レコードのパラメータ領域の並び
    /// （[`Self::build_params`]が割り当てるオフセット順）と一致する。
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
            } else if let ValueKind::StringN(max_len) = param.kind {
                self.gen_string_value_arg(arg, max_len, span);
            } else if matches!(param.kind, ValueKind::Array(_) | ValueKind::Record(_)) {
                self.gen_array_or_record_value_arg(arg, span);
            } else {
                self.gen_expr(arg);
            }
        }
    }

    /// `STRING[n]`値仮引数への実引数のコード生成（Step 18）。
    ///
    /// # タスク0: STRING[n]の値渡しはアドレスを積む（UNCONFIRMED、類推による判断）
    ///
    /// 一次資料（SofTech Microsystems, *UCSD p-System and UCSD Pascal
    /// Version IV: Internal Architecture Guide*）への直接アクセスは本
    /// セッションでも引き続きネットワークegressプロキシにブロックされており
    /// （`docs/research/ucsd-pascal-primary-sources.md`「Step 18セッション」
    /// 節参照）、`STRING[n]`の値渡しの正確な扱いは**UNCONFIRMED**のまま。
    ///
    /// ただし、Step 12で一次資料からCONFIRMED済みの規則
    /// 「VARパラメータおよびレコード・配列値パラメータはアドレスを格納する」
    /// （`crate`モジュールドキュメントの「活性化レコードのレイアウト」、
    /// [`crate::opcode::ConfirmedOp::Rpu`]付近のドキュメント参照）に対し、
    /// `STRING[n]`は（レコード・配列と同様）固定の1ワードに収まらない
    /// 可変長データであるという類推から、本実装は「レコード・配列と同じ
    /// 扱い」を採用する: 値仮引数であってもパラメータ領域には**アドレス**を
    /// 格納する（推測ではなく、この一次資料由来の類推規則に基づく判断で
    /// あることを明記する）。
    ///
    /// 値渡しとしての意味論（呼び出し元の実引数から独立したコピーである
    /// こと）を保つため、渡す値は呼び出し元が新規に確保した一時領域
    /// （グローバルデータ領域の末尾に`1 + max_len`ワード確保。
    /// [`Self::alloc_words`]）へコピーし、その一時領域自身のアドレスを積む
    /// （呼び出し先が仮に書き込んでも、呼び出し元の元の実引数には影響
    /// しない）。
    ///
    /// # スコープ: 文字列リテラル、または直接記憶方式の`STRING[n]`変数のみ
    ///
    /// - 文字列リテラル: [`Self::emit_string_literal_words`]で一時領域へ
    ///   書き込む。
    /// - 単純な識別子で、かつ直接記憶方式（`PROGRAM`直下のグローバル
    ///   `STRING[n]`変数。ローカル`STRING[n]`変数はStep 16から既に
    ///   スコープ外）の`STRING[n]`変数: [`Self::emit_string_copy_words`]で
    ///   一時領域へ1ワードずつコピーする。
    /// - それ以外（既に`STRING[n]`仮引数として受け取った値をさらに別の
    ///   呼び出しへ中継する等）は今回のスコープ外としてエラー報告する。
    fn gen_string_value_arg(&mut self, arg: &Expr, max_len: u8, span: Span) {
        let temp = self.alloc_words(1 + max_len as u16);
        match arg {
            Expr::StringLiteral(text, lit_span) => {
                self.emit_string_literal_words(Level(0), temp, max_len, text, *lit_span, span);
            }
            Expr::Identifier(ident) => {
                let key = normalize(&ident.name);
                match (self.resolve_var(&key), self.lookup_kind(&key)) {
                    (Some(resolved), Some(ValueKind::StringN(_))) if !resolved.indirect => {
                        self.emit_string_copy_words(Level(0), temp, resolved, max_len, span);
                    }
                    (Some(_), Some(ValueKind::StringN(_))) => {
                        self.error(
                            ident.span,
                            "passing an already-received STRING[n] parameter onward as another \
                             STRING[n] value argument is out of scope for this step's minimal \
                             codegen",
                        );
                    }
                    _ => {
                        self.error(
                            ident.span,
                            format!(
                                "'{}' is not a known STRING[n] variable in this scope",
                                ident.name
                            ),
                        );
                    }
                }
            }
            _ => {
                self.error(
                    arg.span(),
                    "STRING[n] value arguments only support a string literal or a simple \
                     STRING[n] variable reference in this step's minimal codegen",
                );
            }
        }
        self.emit(UnconfirmedOp::Lda(Level(0), temp).into(), span);
    }

    fn gen_var_arg(&mut self, arg: &Expr, span: Span) {
        self.gen_identifier_address_arg(
            arg,
            span,
            "cannot pass an expression as a VAR argument (only a simple variable reference is \
             supported by this step's minimal codegen)",
        );
    }

    /// 配列・レコードの値仮引数への実引数のコード生成（Step 21）。
    ///
    /// # UNCONFIRMED/TODO: 値渡しの正しい意味論（コピー生成）は未実装（仮実装）
    ///
    /// 本来のPascalの値渡し意味論では、配列・レコードを値渡しする場合、
    /// 呼び出し側でコピーを作成し、そのコピーのアドレスを渡す（呼び出された
    /// 側での変更が元の変数に影響しない）。しかし本ステップでは実装を
    /// 単純化するため、コピー生成を行わず、[`Self::gen_var_arg`]（`VAR`
    /// 引数）と全く同じ命令列（元の変数のアドレスをそのまま積む）を発行
    /// する。結果として、呼び出された側での配列要素・レコードフィールドへの
    /// 変更が呼び出し側にも反映されてしまう（実質的に参照渡しのように
    /// 振る舞う）という既知の制限がある。これはタスク依頼で明示された
    /// 意図的な仮実装であり、正しい値渡し意味論（コピー生成）は、将来の
    /// `VAR`パラメータの正式な構文・意味論の実装と合わせて別ステップで
    /// 対応する予定（[`Self::build_params`]のドキュメント、リポジトリの
    /// `README.md`「既知の制限」も参照）。
    ///
    /// `STRING[n]`の値渡し（[`Self::gen_string_value_arg`]、Step 18）とは
    /// 対照的に、こちらは意図的にコピーを作らない点に注意。
    ///
    /// # スコープ: 単純な識別子のみ
    ///
    /// [`Self::gen_var_arg`]と同様、単純な変数参照のみサポートする
    /// （配列要素・レコードフィールドを配列/レコード引数として渡すことは
    /// 型上そもそも起こらないため、この制約は実質的に「式は渡せない」
    /// ことのみを意味する）。
    fn gen_array_or_record_value_arg(&mut self, arg: &Expr, span: Span) {
        self.gen_identifier_address_arg(
            arg,
            span,
            "array/record value arguments only support a simple variable reference (not an \
             expression) in this step's minimal codegen",
        );
    }

    /// [`Self::gen_var_arg`]/[`Self::gen_array_or_record_value_arg`]共通の
    /// 実装: 単純な識別子を解決し、そのアドレスをスタックへ積む
    /// （[`Self::gen_address_of_resolved`]）。`arg`が単純な識別子でない場合は
    /// `expr_error`を診断として報告する。
    fn gen_identifier_address_arg(&mut self, arg: &Expr, span: Span, expr_error: &str) {
        let Expr::Identifier(ident) = arg else {
            self.error(arg.span(), expr_error);
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
            // Step 19: 配列要素の読み込み（`arr[i]`をrvalueとして評価）。
            // アドレス計算は代入文の左辺（`Self::gen_assignment`）と共通
            // （`Self::gen_array_element_address`参照）で、読み込みは
            // `VAR`仮引数の間接ロードと同じ`IND`を続けるだけで済む。
            Expr::IndexAccess { array, index, span } => {
                self.gen_array_element_address(array, index, *span);
                self.emit(UnconfirmedOp::Ind.into(), *span);
            }
            // Step 20: レコードフィールドの読み込み（`rec.field`をrvalue
            // として評価）。アドレス解決は代入文の左辺
            // （`Self::gen_assignment`）と共通（`Self::resolve_field_access`
            // 参照）。フィールドのオフセットはコンパイル時定数なので、
            // 配列（`Ind`が必要）と異なり、解決済みの`ResolvedVar`を
            // そのまま`Self::gen_load_resolved`へ渡すだけでよい
            // （`Self::resolve_field_access`のドキュメント「設計判断」参照）。
            Expr::FieldAccess {
                record,
                field,
                span,
            } => match self.resolve_field_access(record, field) {
                Some(resolved) => self.gen_load_resolved(resolved, *span),
                None => {
                    self.emit(UnconfirmedOp::Ldc(0).into(), *span);
                }
            },
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
            // Step 19/20: 配列・レコード全体を1ワードの値として読み込む
            // ことはできない（[`Self::gen_assignment`]の
            // `Some(ValueKind::Array(_))`/`Some(ValueKind::Record(_))`腕の
            // ドキュメント参照。同じ理由でここもガードしないと、配列/
            // レコードの先頭要素・フィールドだけを読む誤ったコードを
            // 黙って生成してしまう）。
            match self.lookup_kind(&key) {
                Some(ValueKind::Array(_)) => {
                    self.error(
                        ident.span,
                        format!(
                            "'{}' is an array; a whole array cannot be used as a value in this \
                             step's minimal codegen (index it first, e.g. '{}[i]')",
                            ident.name, ident.name
                        ),
                    );
                    self.emit(UnconfirmedOp::Ldc(0).into(), ident.span);
                    return;
                }
                Some(ValueKind::Record(_)) => {
                    self.error(
                        ident.span,
                        format!(
                            "'{}' is a record; a whole record cannot be used as a value in \
                             this step's minimal codegen (access a field first, e.g. \
                             '{}.field')",
                            ident.name, ident.name
                        ),
                    );
                    self.emit(UnconfirmedOp::Ldc(0).into(), ident.span);
                    return;
                }
                _ => {}
            }
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
