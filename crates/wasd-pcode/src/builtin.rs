//! `WriteLn`向けの簡略化した組み込みKERNEL呼び出し規約。
//!
//! # 一次資料が示すWriteLn/ReadLnの正体
//!
//! SofTech Microsystems, *UCSD p-System and UCSD Pascal Version IV: Internal
//! Architecture Guide* (First edition, March 1981), Section III.1-III.2
//! （原文p.71-76）によれば、`WriteLn`/`ReadLn`のような言語レベルのI/O呼び出しは
//! **p-machineの専用オペコードではない**。以下の階層構造になっている:
//!
//! ```text
//! 言語レベル (WRITELN, READLN)
//!     ↓ コンパイラ+OSがマッピング
//! Device I/Oルーチン (UNITREAD, UNITWRITE, UNITBUSY, UNITWAIT, UNITCLEAR, UNITSTATUS)
//!     = RSP/IO (Runtime Support Packageの I/O部分)。native codeで実装
//!     ↓
//! BIOS (Basic I/O Subsystem) → 実機のハードウェア制御
//! ```
//!
//! 重要な事実（原文より）:
//! - Device I/Oルーチン（`UNITREAD`/`UNITWRITE`等）はOSの**KERNELユニット**の
//!   ルーチンとして実装され、KERNELは全コンパイル単位から**segment 1**として
//!   常にアクセス可能である（原文: "KERNEL is accessible as segment 1 of every
//!   compilation unit"）。
//! - したがって、コンパイラが`WriteLn`呼び出しをp-codeに変換する際は、
//!   [`crate::opcode::ConfirmedOp::Cxg`]（Call Global External Procedure、
//!   segment番号+procedure番号を指定）を使ってsegment 1（KERNEL）内の
//!   該当ルーチンを呼ぶ形になっているはずである。
//! - デバイス番号（Unit number）: CONSOLE=1, SYSTERM=2, PRINTER=6等が定義済み
//!   （原文 Diagram 2.0）。コンソール出力（`WriteLn`の対象）はUNITNUMBER=1。
//!
//! # 今回の簡略化方針（意図的にUNCONFIRMED）
//!
//! 本格的なRSP/IO・BIOS階層の完全再現（ディスクI/O、実際のシリアル/コンソール
//! ハードウェア制御まで含む）は、本プロジェクトの当面のゴール（Wizardry再現に
//! 向けた最小限のI/O）を大きく超える。そのため、以下の簡略化を採用する:
//!
//! - `KERNEL_SEGMENT`（segment番号1）への`CXG`呼び出しという「形」だけを
//!   一次資料の階層構造から借りるが、実際に呼び出す`procedure`番号
//!   （[`BUILTIN_WRITELN_INT`]等）は、正式な`UNITWRITE`のprocedure番号
//!   （一次資料からは確認できていない）ではなく、**wasd-pcode独自に割り当てた
//!   簡易番号**である。
//! - 正式な`UNITWRITE`が本来要求するはずのパラメータ渡し規約
//!   （device番号・データ領域アドレス・バイト数などをまとめた
//!   「パラメータディスクリプタ」を積む、等）は一切再現しない。
//!   単純に「出力したい値1ワードをスタックへ積んでから呼ぶ」という、
//!   本クレート独自の単純化された呼び出し規約を使う。
//! - これらのprocedure番号自体は**UNCONFIRMED**（wasd-pcode独自の割り当てで
//!   あり、実際のUCSD Pascal処理系とは互換性がない）。正式なRSP/IO呼び出し
//!   規約の完全な実装は、必要になった時点で別ステップとして切り出す
//!   （`crates/pmachine-core/src/machine.rs`の`call_builtin_kernel`
//!   ドキュメントも参照）。

/// KERNELユニットのセグメント番号。全コンパイル単位から`segment 1`として
/// 常にアクセス可能（一次資料 Section III.1-III.2、[`crate::builtin`]の
/// モジュールドキュメント参照）。**CONFIRMED**（セグメント番号1という事実
/// 自体は一次資料に明記されている）。
pub const KERNEL_SEGMENT: u8 = 1;

/// `WriteLn(整数式)`に対応する、wasd-pcode独自の簡易procedure番号。
///
/// # UNCONFIRMED: 正式な`UNITWRITE`のprocedure番号ではない
///
/// 正式なRSP/IOの`UNITWRITE`が実際にどのprocedure番号を持つかは一次資料から
/// 確認できていない。[`crate::builtin`]モジュールドキュメント参照。
pub const BUILTIN_WRITELN_INT: u8 = 1;

/// `WriteLn(Boolean式)`に対応する、wasd-pcode独自の簡易procedure番号。
/// [`BUILTIN_WRITELN_INT`]と同様UNCONFIRMED。
pub const BUILTIN_WRITELN_BOOL: u8 = 2;

/// 引数なし`WriteLn`（改行のみ出力）に対応する、wasd-pcode独自の簡易
/// procedure番号。[`BUILTIN_WRITELN_INT`]と同様UNCONFIRMED。
pub const BUILTIN_WRITELN_NONE: u8 = 3;
