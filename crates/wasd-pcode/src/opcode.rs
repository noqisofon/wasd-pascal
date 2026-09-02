//! p-code命令のオペコード。
//!
//! プロジェクト方針である`Confirmed`/`Unconfirmed`の分離を踏襲する:
//! 一次資料（章・ページ）で番号・オペランド形式・セマンティクスの全てが
//! 確認できたオペコードのみ[`ConfirmedOp`]に置き、それ以外は
//! [`UnconfirmedOp`]に置いて出典コメントと未確認である旨を併記する。

/// 一次資料でオペコード番号・オペランド形式・セマンティクスの全てが
/// 確認済みのオペコード。
///
/// # 沿革: 2026-09時点では0件だった
///
/// このクレートを最初に実装した2026-09時点のセッションでは、サンドボックスの
/// ネットワーク経路（agent proxy）が以下を含む一次資料ホストへの
/// アクセスを全てブロックしていた（`archive.org`, `pascal.hansotten.com`,
/// `www.unige.ch`, `en.wikipedia.org`。プロキシの`recentRelayFailures`で
/// `archive.org:443`への`CONNECT`が"policy denial"で拒否されたことを確認済み）。
/// そのため当時は`ConfirmedOp`が意図的にバリアントを持たない
/// （uninhabited type）ままだった。
///
/// # 2026-09-02: 手続き/関数呼び出し命令を追加
///
/// PROCEDURE/FUNCTION呼び出し（活性化レコード・静的リンク・呼び出し規約）を
/// 追加する本ステップのタスク依頼で、以下の一次資料の該当箇所が
/// 章番号・ページ番号付きで提示された:
///
/// - SofTech Microsystems, *UCSD p-System and UCSD Pascal Version IV:
///   Internal Architecture Guide* (First edition, March 1981)
///   - Section II.4.2.1.3（活性化レコードの構造、p.48-49）
///   - Section II.4.2.2.18（呼び出し命令、p.65-67）
///
/// これに基づき、同一セグメント内呼び出し（`PROCEDURE`/`FUNCTION`の通常
/// 呼び出し）に必要な6命令（[`Cpl`](ConfirmedOp::Cpl)/
/// [`Cpg`](ConfirmedOp::Cpg)/[`Cpi`](ConfirmedOp::Cpi)/
/// [`Scpi1`](ConfirmedOp::Scpi1)/[`Scpi2`](ConfirmedOp::Scpi2)/
/// [`Rpu`](ConfirmedOp::Rpu)）を追加する。
///
/// ## 重要な注記: このセッション自身は一次資料に直接あたれていない
///
/// このセッションのサンドボックスのネットワーク経路（agent proxy）は、本
/// ステップの実装開始時点でも`archive.org`へのアクセスを引き続き
/// ブロックしていた（`WebFetch`で
/// `https://archive.org/details/UCSD_P-System_UCSD_PASCAL_Internal_Architecture_Guide`
/// への到達を試み、`EGRESS_BLOCKED`エラーを確認済み）。そのため、上記の
/// 章番号・ページ番号・オペコード番号（144/145/239/240/146/150）は
/// **このセッションが一次資料に直接あたって独立に検証したものではなく、
/// タスク依頼の記述としてそのまま提示された情報を採用したもの**である
/// （おそらく、このセッションとは別に一次資料へアクセスできたセッションの
/// 調査結果に基づくと推測されるが、その調査自体をこのセッションで再現・
/// 検証してはいない）。虚偽の「独立に確認済み」という表示を避けるため、
/// この経緯をここに明記する。
///
/// # 2026-09-02（Step 14）: `CXG`の追加、比較命令・ジャンプ命令・`RPU`のさらなる確認
///
/// `WriteLn`の実装方針を調査するタスク依頼で、Section II.4.2.2の命令表
/// （全命令一覧）とSection III.1-III.2（I/O階層）が原文引用付きで新たに
/// 提示された。これにより[`ConfirmedOp::Cxg`]を追加し、[`UnconfirmedOp::Equ`]
/// （比較命令）・[`CodeAddress`]（ジャンプ命令のオフセット解釈）・
/// [`ConfirmedOp::Rpu`]（`b`パラメータ）に残っていたUNCONFIRMED事項を
/// 解消できた（各バリアントのドキュメント参照）。
///
/// ## このセッションも一次資料に直接あたってはいない
///
/// 上記と同じ注記が今回も当てはまる: このセッション自身が`archive.org`等の
/// 一次資料ホストへ`WebFetch`等で直接アクセスして検証したわけではなく、
/// タスク依頼の記述として提示された原文引用（Section II.4.2.2.13/17/18、
/// III.1-III.2）をそのまま採用した。独立検証ではない旨をここに明記する。
///
/// 加えて、これらのオペコード番号の値そのもの（144等）はIR上のデータとしては
/// 保持しない。[`crate`]のドキュメントの通り、実バイナリへのエンコード
/// （オペコード番号をバイト列へ落とす処理）は本クレートのスコープ外
/// （将来の実バイナリ生成ステップ）である。また、実機の呼び出し命令が
/// 本来持つ「プロシージャ番号(UB)→プロシージャ辞書経由の間接参照」も、
/// [`CodeAddress`]が実バイト列オフセットの代わりに命令列インデックスを
/// 保持しているのと同様、本IRでは呼び出し先を直接[`CodeAddress`]として
/// 保持する簡略化を採用している（各バリアントのドキュメント参照）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmedOp {
    /// `CPL <target>`: Call Local Procedure。オペコード144。現在実行中の
    /// プロシージャの直接の子で、同一セグメント内のプロシージャを呼ぶ。
    /// 新しいマーク・スタック制御ワード（活性化レコード先頭の5ワード）の
    /// 静的リンク（`MSSTAT`）は、呼び出し元自身の活性化レコード
    /// （old MP）に設定される。
    ///
    /// 本ステップの`PROCEDURE`/`FUNCTION`は常に`PROGRAM`直下（lexレベル1）
    /// にのみ宣言できるという`wasd_ast`の制約（`ProcDecl`/`FuncDecl`は
    /// 自身の中にさらに`proc_decls`/`func_decls`を持たない）により、
    /// [`crate::codegen::CodeGenerator`]が実際に発行する呼び出し命令は
    /// 常に[`ConfirmedOp::Cpg`]であり、この`Cpl`は使われない。将来
    /// `PROCEDURE`内`PROCEDURE`のようなさらに深いネストがASTに追加された
    /// 場合に備えた設計上の受け皿として用意してある。
    Cpl(CodeAddress),
    /// `CPG <target>`: Call Global Procedure。オペコード145。呼び出し先の
    /// lexレベルは常に1（`PROGRAM`直下に宣言された`PROCEDURE`/
    /// `FUNCTION`）で、同一セグメント内のプロシージャを呼ぶ。新しい
    /// `MSSTAT`（静的リンク）はBASE（プログラム本体の活性化レコード）に
    /// 設定される。
    ///
    /// 本ステップのASTの制約（[`Cpl`]のドキュメント参照）により、
    /// `PROCEDURE`/`FUNCTION`呼び出しは（呼び出し元がプログラム本体・
    /// 他のプロシージャ本体のいずれであっても、また再帰呼び出しであっても）
    /// 常にこの`Cpg`として発行される。
    Cpg(CodeAddress),
    /// `CPI <db> <target>`: Call Intermediate Procedure。オペコード146。
    /// 現在実行中のプロシージャよりDBレベル低いlexレベルの、同一セグメント
    /// 内のプロシージャを呼ぶ。
    ///
    /// 本ステップのASTは`PROGRAM`直下（lexレベル1）を超えるネストを
    /// 表現できないため（[`Cpl`]のドキュメント参照）、この命令は現時点の
    /// コード生成では発行されない。
    Cpi(u8, CodeAddress),
    /// `SCPI1 <target>`: Short Call Intermediate（DB=1版）。オペコード239。
    /// 静的チェーンを呼び出し環境の親に設定してプロシージャを呼ぶ、
    /// [`Cpi`]のDB=1専用の短縮形。[`Cpi`]のドキュメント参照。現時点では
    /// 発行されない。
    Scpi1(CodeAddress),
    /// `SCPI2 <target>`: Short Call Intermediate（DB=2版、祖父母環境）。
    /// オペコード240。[`Scpi1`]/[`Cpi`]のドキュメント参照。現時点では
    /// 発行されない。
    Scpi2(CodeAddress),
    /// `RPU <b>`: Return from Procedure。オペコード150。呼び出し元の状態を
    /// マーク・スタック制御ワード（MSCW）から復元し、MSCWをスタックから
    /// pop する。加えて`b`ワード分スタックを切り詰める（関数の戻り値が
    /// ある場合、それは切り詰め対象の範囲より上に積まれているため残る）。
    ///
    /// # `b`の計算式: 方針AがCONFIRMED（一次資料原文 + Step 13の実行検証の両方）
    ///
    /// 一次資料原文（Section II.4.2.2.18）に明確な記述がある:
    ///
    /// ```text
    /// RPU  150 B  <activation>:<func>
    ///     Return from Procedure. Restore state of calling procedure from MSCW
    ///     and discard. Pop MSCW from Stack. Cut back an additional B words from
    ///     Stack, leaving function value, if appropriate.
    /// ```
    ///
    /// つまり: (1) MSCW（5ワード）をスタックからpopし、(2) さらに`B`ワード分
    /// スタックを切り詰め、(3) `FUNCTION`の場合は戻り値だけが残るよう
    /// 切り詰める。`B`は「ローカル変数＋パラメータ領域の合計ワード数」
    /// （活性化レコードのMSCW以外の部分）に相当する。これは
    /// [`crate::codegen::CodeGenerator`]の`emit_rpu`が採用する方針A
    /// （`b` = `DATASIZE` + パラメータ領域のワード数）と一致する。
    ///
    /// Step 13で`pmachine-core`（p-machineインタプリタ）を実装し、
    /// `PROCEDURE`/`FUNCTION`呼び出しを実際に実行した上で、呼び出し前後の
    /// スタックポインタ（SP）が期待通りの位置（`PROCEDURE`なら呼び出し前と
    /// 同じ、`FUNCTION`なら戻り値の1ワード分だけ大きい）に戻ることを確認
    /// した（`pmachine-core/tests/rpu_b_verification.rs`参照。単純呼び出し・
    /// 値引数・`VAR`引数・複数回の呼び出し・再帰呼び出しの組み合わせで
    /// 検証済み）。よって方針Aは**本プロジェクトの実行モデル内では
    /// CONFIRMED**として扱ってよい。ただし、これは実機バイナリでの検証
    /// ではなく、あくまで本プロジェクトの簡略化されたIR・
    /// `pmachine-core`のインタプリタ実装内での自己無矛盾性の検証である点に
    /// 注意（`pmachine-core`のクレートドキュメントの「メモリモデル」
    /// 「呼び出し規約」の各節を参照。特に、活性化レコード先頭のマーク・
    /// スタック制御ワードは`pmachine-core`では実データとして`stack`上には
    /// 確保せず、別テーブルとして持たせる簡略化を採用している）。
    Rpu(u16),
    /// `CXG <seg>, <proc>`: Call Global External Procedure。オペコード148。
    /// 現在実行中のセグメントとは異なるセグメント`seg`の、そのセグメント
    /// 内でグローバルな（lexレベル1の）プロシージャ`proc`を呼ぶ。
    ///
    /// # CONFIRMED: オペコード番号・大まかなセマンティクス
    ///
    /// SofTech Microsystems, *UCSD p-System and UCSD Pascal Version IV:
    /// Internal Architecture Guide* (First edition, March 1981), Section
    /// II.4.2.2.18で確認済み。
    ///
    /// # 本クレートでの用途: `WriteLn`向けの簡略化したKERNEL呼び出し
    ///
    /// 一次資料（Section III.1-III.2）によれば、`WriteLn`/`ReadLn`のような
    /// 言語レベルのI/O呼び出しはp-machineの専用オペコードではなく、
    /// コンパイラ+OSがKERNELユニット（全コンパイル単位から`segment 1`として
    /// 常にアクセス可能）の`UNITWRITE`/`UNITREAD`ルーチン呼び出しに変換する、
    /// という階層構造になっている。本クレートは`WriteLn`をこの`CXG`命令で
    /// KERNELセグメント（[`crate::builtin::KERNEL_SEGMENT`]）内の簡易
    /// procedure番号（[`crate::builtin`]モジュールドキュメント参照）を
    /// 呼び出す形で表現する。
    ///
    /// ただし、正式な`UNITWRITE`呼び出しが本来必要とするパラメータ
    /// ディスクリプタ等の呼び出し規約は一切再現しておらず、あくまで
    /// 「セグメント番号+procedure番号を指定して呼ぶ」という`CXG`の形だけを
    /// 借りた簡略化である点に注意（[`crate::builtin`]モジュール
    /// ドキュメント、および`pmachine-core`の`call_builtin_kernel`ドキュメント
    /// 参照）。呼び出し先はKERNELの組み込みエミュレーションのみであり、
    /// 通常の`PROCEDURE`/`FUNCTION`のような活性化レコード（マーク・
    /// スタック・ローカル変数領域等）は一切組み立てない・`RPU`で戻ることも
    /// ない、という点で[`Cpl`]/[`Cpg`]/[`Cpi`]とは実行モデルが大きく異なる
    /// （`pmachine-core`側の実装判断。呼び出し前にスタックへ積んだ引数を
    /// そのままKERNELエミュレーションが消費し、呼び出し命令の直後へ制御が
    /// 戻るのみ）。
    Cxg(u8, u8),
}

/// データ領域中の1ワードのアドレス（ワード単位のオフセット）。
///
/// [`Level`]が0（グローバルスコープ、または現在実行中のプロシージャ/
/// プログラム自身のフレーム）の場合はグローバルデータ領域中のオフセット、
/// [`Level`]が1以上の場合はその静的リンク先の活性化レコード内の
/// オフセット（マーク・スタックの5ワードを含む、活性化レコード先頭
/// からのワード数。[`crate::codegen::CodeGenerator`]のドキュメント参照）
/// を表す。
///
/// # UNCONFIRMED: 絶対アドレスの原点
///
/// 実機のp-Systemデータ領域における実際の原点オフセットは採用していない。
/// ここでは0番地起点の相対オフセットとして扱い、実際の絶対配置は将来の
/// （本ステップの対象外である）実行時リンク処理に委ねる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address(pub u16);

/// 静的リンク（レキシカルネスト）のレベル差。0 = 現在実行中の
/// プロシージャ/プログラム自身のフレーム、1 = そのプロシージャの静的な親
/// （1つ外側のレキシカルスコープ。本ステップのスコープでは常に
/// `PROGRAM`本体、すなわちグローバルスコープ）のフレーム、というように、
/// [`UnconfirmedOp::Lod`]/[`UnconfirmedOp::Str`]/[`UnconfirmedOp::Lda`]が
/// 実行時に何回静的リンクを辿るかを表す。
///
/// # UNCONFIRMED
///
/// 本ステップで`LOD`/`STR`にこのレベル差オペランドを追加した
/// （[`Address`]の以前の版のドキュメントに残っていた申し送り事項の解消）。
/// ただし、このオペランドの正確なビット幅・エンコーディングは
/// [`UnconfirmedOp`]の他のオペランドと同様未確認である。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Level(pub u8);

/// p-code命令列中の1命令の位置（命令列のインデックス）。
///
/// # CONFIRMED: 分岐命令は単純な相対バイトオフセット加算方式（JTAB等の間接テーブルは存在しない）
///
/// 一次資料（SofTech Microsystems, *UCSD p-System and UCSD Pascal Version
/// IV: Internal Architecture Guide*, Section II.4.2.2.17）に、分岐命令の
/// 原文が以下の通り確認できた:
///
/// ```text
/// UJP  138 SB  <>:<>       Unconditional Jump. Jump by byte offset SB.
/// FJP  212 SB  <Bool>:<>   False Jump. Jump by byte offset SB if TOS is false.
/// TJP  241 SB  <Bool>:<>   True Jump. Jump by byte offset SB if TOS is true.
/// JPL  139 W   <>:<>       Unconditional Long Jump. Jump W bytes from current location.
/// FJPL 213 W   <Bool>:<>   False Long Jump. Jump W bytes from current location if TOS is false.
/// ```
///
/// 「Jump by byte offset SB」「Jump W bytes from current location」という
/// 記述のみで、二次資料（markbessey.blog等）が示唆していたような`JTAB`
/// （間接ジャンプテーブル）方式への言及は一切ない。単純な相対オフセット
/// 加算方式という、以前からこのIRが採用していた解釈がCONFIRMEDとなった。
///
/// ただし、これはあくまで「オフセットの意味論（正負を問わず単純加算）」が
/// 確認できたということであり、本IRの`CodeAddress`が実バイト列オフセット
/// ではなく`Vec<Instruction>`中のインデックス（命令番号）を分岐先として
/// 保持するという抽象化自体は、実バイナリ生成が今回のスコープ外である
/// ことに変わりないため、意図的に維持している。実際のバイト列へのエン
/// コード（`SB`/`W`のバイト幅、命令自身の直後からのオフセットかどうかの
/// 厳密な起点等）は、今回のスコープ外である「実バイナリ生成」ステップで
/// 改めて確認すること。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CodeAddress(pub u32);

/// 一次資料で確認できていない、または実装者の推測が含まれるオペコード。
///
/// # UNCONFIRMED: 出典と未確認である旨
///
/// ここに定義した各バリアントのニーモニックは、UCSD p-System由来のp-code
/// 命令セットとして一般に（二次資料・伝聞レベルで）知られている名称
/// （`LOD`/`STR`/`LDC`/`ADI`/`SBI`/`MPI`/`DVI`等）を採用しており、
/// 「実装者が独自に創作した名称」ではない。ただし以下は
/// **このセッションでは一次資料に基づき確認できていない**:
///
/// - 各命令の正確なオペコード番号（16進数/10進数）
/// - オペランドのバイト数・エンコーディング（即値がインラインか、
///   後続バイト列か）
/// - `LOD`/`STR`が本来持つはずの「レベル差」オペランド（[`Address`]の
///   ドキュメント参照）
/// - 比較命令（`EQU`/`NEQ`/`LEQ`/`GEQ`）が実機ではオペランド型ごとに
///   別々のオペコード番号を持つ（例: `EQUI`/`EQUR`/`EQUB`等）ことが一般的に
///   知られているが、本実装はスコープがINTEGER/BOOLEANの2型のみであり、
///   かつ両者ともp-machine上では1ワードの値として同じ表現になると仮定し、
///   型ごとに分けず単一のバリアントで代表させている（この簡略化自体も
///   UNCONFIRMED。ただし`EQUI`/`NEQI`/`LEQI`/`GEQI`というINTEGER版の
///   オペコード番号自体はCONFIRMED。[`Leq`]/[`Geq`]のドキュメント参照）。
///   なお、`<`/`>`（strictな「より小さい」「より大きい」）に対応する
///   オペコードは一次資料に**存在しない**（CONFIRMED。[`Leq`]/[`Geq`]の
///   ドキュメント参照）。本実装はこれらを`LEQI`/`GEQI`と`LNOT`相当の
///   [`Not`]の組み合わせで合成する。
/// - 論理演算の正確なニーモニック表記（`IOR`か`LOR`か等。ここでは`IOR`を
///   採用しているが未確認）。
/// - `BOOLEAN`の`TRUE`/`FALSE`のワード表現（ここでは`TRUE = 1`/
///   `FALSE = 0`という一般的な慣行を仮定しているが未確認）。
/// - プログラム終了命令の正確なニーモニック（ここでは`STP`を採用して
///   いるが未確認）。
///
/// 参照すべき一次資料（未確認、次回セッションでの確認待ち。
/// [`ConfirmedOp`]のドキュメント参照）:
/// - SofTech Microsystems, *UCSD p-System and UCSD Pascal Version IV:
///   Internal Architecture Guide* (First edition, March 1981)
/// - T. Nouspikel's TI-99/4A p-System実装ガイド
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnconfirmedOp {
    /// `LDC <value>`: 16bit整数即値をスタックへ積む（load constant）。
    Ldc(i16),
    /// `LOD <level>, <addr>`: `level`個の静的リンクを辿った先の活性化
    /// レコード（`level`が0ならグローバルデータ領域）の`addr`番地の1ワードを
    /// スタックへ積む（load）。
    ///
    /// このワードが`VAR`仮引数（参照渡し）のスロットである場合、そこに
    /// 格納されているのは値そのものではなく呼び出し元の変数の
    /// **アドレス**である点に注意（[`Lda`]/[`Ind`]のドキュメント参照）。
    /// その場合、実際の値を読むにはこの命令に続けて[`Ind`]を発行する
    /// 必要がある（[`crate::codegen::CodeGenerator`]の
    /// `gen_load_resolved`のドキュメント参照）。
    Lod(Level, Address),
    /// `STR <level>, <addr>`: スタック最上段の1ワードを、`level`個の静的
    /// リンクを辿った先の活性化レコード（`level`が0ならグローバルデータ
    /// 領域）の`addr`番地へ格納し、スタックから取り除く（store）。
    ///
    /// [`Lod`]と同様、対象が`VAR`仮引数のスロットである場合は
    /// このアドレス自体を書き換えてしまうことになり、参照先の変数へは
    /// 書き込めない点に注意（そちらへ書き込みたい場合は[`Sti`]を使う）。
    Str(Level, Address),
    /// `LDA <level>, <addr>`（Load Address）: `level`個の静的リンクを
    /// 辿った先の活性化レコードの`addr`番地の**アドレス**をスタックへ
    /// 積む（値そのものではない）。`VAR`仮引数として変数を渡す際に、
    /// 呼び出し元がその変数のアドレスを積むために使う。
    ///
    /// # UNCONFIRMED: ニーモニック・オペコード・存在そのもの
    ///
    /// タスク依頼の記述で名前が挙がっていた`LAO`（Load Address of...、
    /// おそらくグローバル変数専用の短縮形）とは意図的に別の、より一般的な
    /// （レベル差を取れる）命令として定義した。`VAR`仮引数の受け渡しには
    /// 「グローバル変数だけでなく、呼び出し元のどのレベルの変数の
    /// アドレスも積める」命令が必要だが、この一般形の正確な名称・
    /// オペコード番号・（`LAO`という短縮形と併存するのか、それとも`LAO`
    /// 自身が実はレベル差を取れる一般形なのか）は、このセッションでは
    /// 一次資料に一切あたれておらず全くの未確認。このセッションでの
    /// 実装上の都合による暫定的な命名・設計であることを明記する。
    Lda(Level, Address),
    /// `IND`（Load Indirect）: スタック最上段の1ワードをアドレスとして
    /// pop し、そのアドレスの指す1ワードをスタックへ積む（間接ロード）。
    /// `VAR`仮引数のスロット（[`Lod`]で読むとアドレスが得られる）の
    /// 指す先の実際の値を読むために、[`Lod`]に続けて使う。
    ///
    /// # UNCONFIRMED
    ///
    /// [`Lda`]と同様、このセッションでは一次資料に一切あたれておらず、
    /// 命令の存在・名称・オペコード番号のいずれも未確認。`VAR`仮引数
    /// （参照渡し）の意味論上、呼び出し先が参照先の値を読み書きするには
    /// 何らかの間接アドレッシング命令が不可欠であるため、実装上の都合で
    /// 導入した。
    Ind,
    /// `STI`（Store Indirect）: スタックに`[..., address, value]`の順
    /// （`address`が下、`value`が上）で積まれている状態から、`value`を
    /// `address`の指す1ワードへ格納し、両方をスタックから取り除く
    /// （間接ストア）。`VAR`仮引数を通じて呼び出し元の変数へ書き込む際に
    /// 使う（[`crate::codegen::CodeGenerator`]の`gen_store_resolved`の
    /// ドキュメント参照: まず[`Lod`]でスロットからアドレスを積み、続けて
    /// 格納したい値を積んでから、この`STI`を発行する）。
    ///
    /// # UNCONFIRMED
    ///
    /// [`Ind`]と同様、命令の存在・名称・オペコード番号に加え、スタック上の
    /// `address`と`value`の積み順（本実装では「アドレスが先、値が後」と
    /// 仮定した）自体も、このセッションでは一次資料に一切あたれておらず
    /// 全くの未確認。
    Sti,
    /// `ADI`: 整数の加算（add integer）。
    Adi,
    /// `SBI`: 整数の減算（subtract integer）。
    Sbi,
    /// `MPI`: 整数の乗算（multiply integer）。
    Mpi,
    /// `DVI`: 整数の除算（`DIV`、divide integer）。
    Dvi,
    /// `MOD`: 整数の剰余。
    Mod,
    /// `NGI`: 整数の符号反転（negate integer、単項マイナス）。
    Ngi,
    /// `EQU`: 等しい。
    ///
    /// # CONFIRMED: `EQUI`/`NEQI`/`LEQI`/`GEQI`のオペコード番号
    ///
    /// 一次資料（SofTech Microsystems, *UCSD p-System and UCSD Pascal
    /// Version IV: Internal Architecture Guide*, Section II.4.2.2.13）に
    /// INTEGER比較命令の一覧が確認できた:
    ///
    /// ```text
    /// EQUI 176  TOS-1 = TOS
    /// NEQI 177  TOS-1 <> TOS
    /// LEQI 178  TOS-1 <= TOS
    /// GEQI 179  TOS-1 >= TOS
    /// ```
    ///
    /// **`<`/`>`（strictな「より小さい」「より大きい」）に対応するオペコード
    /// はこの一覧に存在しない。** これはCONFIRMEDな事実である
    /// （[`crate::codegen::CodeGenerator::emit_binop`]のドキュメント参照:
    /// `a < b`は`NOT (a >= b)`として`GEQI`+`LNOT`相当の[`Not`]の組み合わせで、
    /// `a > b`は`NOT (a <= b)`として`LEQI`+[`Not`]の組み合わせで、それぞれ
    /// 合成する）。本バリアント自体のオペコード番号（176相当）・型ごとの
    /// 分離（`EQUI`のようなINTEGER専用の型サフィックス）は本IRのデータ
    /// としては保持しない簡略化を採用している（[`ConfirmedOp`]のドキュメント
    /// 「加えて、これらのオペコード番号の値そのもの...」の節と同じ方針）。
    Equ,
    /// `NEQ`: 等しくない。[`Equ`]のドキュメント参照（`NEQI` 177、CONFIRMED）。
    Neq,
    /// `LEQ`: 以下。[`Equ`]のドキュメント参照（`LEQI` 178、CONFIRMED）。
    Leq,
    /// `GEQ`: 以上。[`Equ`]のドキュメント参照（`GEQI` 179、CONFIRMED）。
    Geq,
    /// `AND`: 論理積。
    And,
    /// `IOR`: 論理和。
    Ior,
    /// `NOT`: 論理否定。
    Not,
    /// `UJP <target>`: 無条件分岐（unconditional jump）。
    Ujp(CodeAddress),
    /// `FJP <target>`: スタック最上段の`BOOLEAN`が偽の場合のみ分岐する
    /// （false jump）。真偽いずれの場合もスタックからは取り除かれる。
    Fjp(CodeAddress),
    /// `STP`: プログラムの実行を終了する（stop）。
    Stp,
}

/// p-code命令のオペコード。`Confirmed`/`Unconfirmed`の分離については
/// このモジュールのドキュメントを参照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Confirmed(ConfirmedOp),
    Unconfirmed(UnconfirmedOp),
}

impl Opcode {
    /// 分岐先・呼び出し先アドレスを持つ命令（`UJP`/`FJP`/`CPL`/`CPG`/`CPI`/
    /// `SCPI1`/`SCPI2`）であれば、そのアドレスへの可変参照を返す。
    /// バックパッチ（[`crate::codegen::CodeGenerator`]のドキュメント参照）
    /// に使う。`RPU`はアドレスを持たないため対象外。
    pub fn jump_target_mut(&mut self) -> Option<&mut CodeAddress> {
        match self {
            Opcode::Unconfirmed(UnconfirmedOp::Ujp(target) | UnconfirmedOp::Fjp(target)) => {
                Some(target)
            }
            Opcode::Confirmed(
                ConfirmedOp::Cpl(target)
                | ConfirmedOp::Cpg(target)
                | ConfirmedOp::Cpi(_, target)
                | ConfirmedOp::Scpi1(target)
                | ConfirmedOp::Scpi2(target),
            ) => Some(target),
            _ => None,
        }
    }
}

impl From<UnconfirmedOp> for Opcode {
    fn from(op: UnconfirmedOp) -> Self {
        Opcode::Unconfirmed(op)
    }
}

impl From<ConfirmedOp> for Opcode {
    fn from(op: ConfirmedOp) -> Self {
        Opcode::Confirmed(op)
    }
}
