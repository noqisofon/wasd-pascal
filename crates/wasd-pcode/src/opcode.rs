//! p-code命令のオペコード。
//!
//! プロジェクト方針である`Confirmed`/`Unconfirmed`の分離を踏襲する:
//! 一次資料（章・ページ）で番号・オペランド形式・セマンティクスの全てが
//! 確認できたオペコードのみ[`ConfirmedOp`]に置き、それ以外は
//! [`UnconfirmedOp`]に置いて出典コメントと未確認である旨を併記する。

/// 一次資料でオペコード番号・オペランド形式・セマンティクスの全てが
/// 確認済みのオペコード。
///
/// # 現状: 0件（意図的にバリアントを持たない）
///
/// このクレートを実装した2026-09時点のセッションでは、サンドボックスの
/// ネットワーク経路（agent proxy）が以下を含む一次資料ホストへの
/// アクセスを全てブロックしていた（`archive.org`, `pascal.hansotten.com`,
/// `www.unige.ch`, `en.wikipedia.org`。プロキシの`recentRelayFailures`で
/// `archive.org:443`への`CONNECT`が"policy denial"で拒否されたことを確認済み）。
/// そのため、
///
/// - SofTech Microsystems, *UCSD p-System and UCSD Pascal Version IV:
///   Internal Architecture Guide* (First edition, March 1981)
/// - T. Nouspikel's TI-99/4A p-System実装ガイド
///
/// のいずれにもこのセッションでは直接あたれず、章番号・ページ番号付きで
/// 「確認済み」と言えるオペコードが1件も無い。この事実を誤魔化さず、
/// この`enum`は意図的にバリアントを持たない（uninhabited type）ままにする。
///
/// この判断は、実装を止めて確認を求めるべきというプロジェクト方針に
/// 従い、ユーザーに状況を説明した上で「一次資料アクセスが復旧するまで
/// 全命令をUnconfirmedとして進めてよい」という明示的な承認を得て行った
/// （2026-09-02のセッション対話）。
///
/// 将来、一次資料の該当章・ページ番号を実際に確認できたオペコードから、
/// [`UnconfirmedOp`]の対応するバリアントをここへ1つずつ移すこと。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmedOp {}

/// グローバルデータ領域中の1ワードのアドレス（ワード単位のオフセット）。
///
/// # UNCONFIRMED: 絶対アドレスの原点、および`LOD`/`STR`本来の「レベル差」操作
///
/// - 実機のp-Systemデータ領域における実際の原点オフセット（スタックマーク
///   等のために低位アドレスが予約されているはずだが、その正確な値は
///   未確認）は採用していない。ここでは0番地起点の相対オフセットとして
///   扱い、実際の絶対配置は将来の（本ステップの対象外である）実行時
///   リンク処理に委ねる。
/// - `LOD`/`STR`は本来、現在の静的リンク（レキシカルネスト）レベルと
///   目的の変数が宣言されたレベルとの差（レベル差）をオペランドに持つ
///   はずだが、今回のスコープは`PROCEDURE`/`FUNCTION`を含まない単一の
///   `PROGRAM`本体のみであり、ネストしたレベルが存在しないため、
///   レベルオペランドをまだ実装していない。次のステップで
///   `PROCEDURE`/`FUNCTION`を追加する際に、一次資料を確認の上で
///   追加すること。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address(pub u16);

/// p-code命令列中の1命令の位置（命令列のインデックス）。
///
/// # UNCONFIRMED: 分岐命令のオフセットの基準点・エンコーディング
///
/// 実機の`UJP`/`FJP`が分岐先をどう符号化するか（命令列先頭からのバイト
/// オフセットか、分岐命令自身の直後からの相対オフセットか等）は未確認。
/// このIRでは、そのバイトレベルのエンコーディングを決定する前段階の
/// 抽象として、単に`Vec<Instruction>`中のインデックス（命令番号）を
/// 分岐先として保持する。実際のバイト列へのエンコードは、今回のスコープ
/// 外である「実バイナリ生成」ステップで、一次資料を確認した上で行うこと。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
/// - 比較命令（`EQU`/`NEQ`/`LES`/`LEQ`/`GRT`/`GEQ`）が実機ではオペランド
///   型ごとに別々のオペコード番号を持つ（例: `EQUI`/`EQUR`/`EQUB`等）
///   ことが一般的に知られているが、本実装はスコープがINTEGER/BOOLEANの
///   2型のみであり、かつ両者ともp-machine上では1ワードの値として
///   同じ表現になると仮定し、型ごとに分けず単一のバリアントで代表させて
///   いる（この簡略化自体もUNCONFIRMED）。
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
    /// `LOD <addr>`: グローバルデータ領域の1ワードをスタックへ積む（load）。
    Lod(Address),
    /// `STR <addr>`: スタック最上段の1ワードをグローバルデータ領域へ
    /// 格納し、スタックから取り除く（store）。
    Str(Address),
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
    Equ,
    /// `NEQ`: 等しくない。
    Neq,
    /// `LES`: より小さい。
    Les,
    /// `LEQ`: 以下。
    Leq,
    /// `GRT`: より大きい。
    Grt,
    /// `GEQ`: 以上。
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
    /// 分岐先アドレスを持つ命令（`UJP`/`FJP`）であれば、そのアドレスへの
    /// 可変参照を返す。バックパッチ（[`crate::codegen::CodeGenerator`]の
    /// ドキュメント参照）に使う。
    pub fn jump_target_mut(&mut self) -> Option<&mut CodeAddress> {
        match self {
            Opcode::Unconfirmed(UnconfirmedOp::Ujp(target) | UnconfirmedOp::Fjp(target)) => {
                Some(target)
            }
            _ => None,
        }
    }
}

impl From<UnconfirmedOp> for Opcode {
    fn from(op: UnconfirmedOp) -> Self {
        Opcode::Unconfirmed(op)
    }
}
