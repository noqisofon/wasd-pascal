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

/// `WriteLn(文字列リテラル)`に対応する、wasd-pcode独自の簡易procedure番号。
/// [`BUILTIN_WRITELN_INT`]と同様UNCONFIRMED。
///
/// # 文字列定数の扱い: Constant Poolを模した`string_pool`
///
/// Internal Architecture Guide, Section II.2.1.4 "The Constant Pool"に
/// よれば、1ワードに収まらない定数（文字列を含む）はp-codeの命令列とは
/// 別の「Constant Pool」領域に格納され、`LCO`（Load Constant Offset）
/// 命令でそこへのオフセットを指定して参照する。この呼び出し（
/// `WriteLn('...')`）でも同じ発想を借り、文字列定数は命令列に埋め込まず
/// [`crate::ir::PCodeModule::string_pool`]という別テーブルに集約する。
///
/// ただし本クレートは`LCO`の正確なエンコーディング（オフセットの単位が
/// バイトかワードか、Constant Pool自体のバイナリレイアウト等）を一次資料
/// から完全には確認できていないため、それを再現することはしない。実際に
/// 使うのは「文字列を`string_pool`に追加してインデックス（`usize`）を得て、
/// そのインデックスをスタックへ積む（本クレートは`LDC`命令をそのまま
/// 転用する。他に専用の即値ロード命令を持たないため）」という、
/// **コンパイラ（本クレート）とVM（`pmachine-core`）の間だけで通用する
/// 自前の簡略化されたプロトコル**である。「スタック上のこの値は
/// 文字列プールへのインデックスである」という意味づけ自体は命令からは
/// 読み取れず、`BUILTIN_WRITELN_STRING`という呼び出し先の番号だけが
/// それを暗黙に約束している点に注意（[`crate::codegen::CodeGenerator::gen_writeln_call`]
/// 参照）。
pub const BUILTIN_WRITELN_STRING: u8 = 4;

/// `WriteLn(STRING[n]変数)`に対応する、wasd-pcode独自の簡易procedure番号。
/// [`BUILTIN_WRITELN_INT`]と同様UNCONFIRMED。
///
/// # Step 16: `STRING[n]`変数の中身をランタイムで読む
///
/// [`BUILTIN_WRITELN_STRING`]（文字列リテラル）とは異なり、`STRING[n]`
/// 変数の中身はコンパイル時には分からない（実行時に代入された値による）。
/// そのため、コンパイル時に確定する`string_pool`インデックスではなく、
/// 変数自身の**アドレス**（[`crate::opcode::UnconfirmedOp::Lda`]で積む）を
/// スタックへ積んでこのprocedureを呼ぶ（
/// [`crate::codegen::CodeGenerator::gen_writeln_call`]参照）。
///
/// 呼び出された側（`pmachine-core`の`call_builtin_kernel`）は、popした
/// アドレスの指す1ワードを「長さ」として読み、続く「長さ」ワード分を
/// 文字コード（1ワード=1文字）として読んで出力する。
///
/// # メモリレイアウト: 「1ワード=1文字」という単純化（UNCONFIRMED）
///
/// UCSD Pascalの`STRING[n]`は一次資料の慣習として「先頭1バイト＝長さ、
/// 続く最大`n`バイトが文字データ」という**バイト単位**のレイアウトを持つと
/// 理解しているが、UCSD p-System固有の一次資料での確証は得られていない
/// （`docs/research/ucsd-pascal-primary-sources.md`のStep 16セッション節
/// 参照。archive.org等の一次資料ホストへのアクセスがすべてネットワーク
/// egressプロキシにブロックされ、直接確認できなかった）。
///
/// 加えて、p-machineは16ビットワード単位でアドレッシングする
/// （[`crate::opcode::Address`]のドキュメント参照）ため、本クレートの
/// 既存の`LOD`/`STR`は1ワード単位でしか読み書きできず、1ワードに2文字を
/// 詰める本来のバイトパッキング（未確認）や、ワード内の特定バイトだけを
/// 読み書きする命令は一切実装していない。新規命令を安易に創作しない
/// という方針（`crates/wasd-pcode/src/opcode.rs`モジュールドキュメント
/// 参照）のもと、既存の`LDC`/`LOD`/`STR`/`LDA`だけで実装できる
/// 「長さ1ワード + 文字ごとに1ワード（`max_len`ワード分を常に確保）」
/// という単純化されたレイアウトを採用する。これは実際のUCSD p-System
/// のバイト単位レイアウトを再現するものでは**ない**（本クレートが既に
/// 採用している「`WriteLn`はKERNELへの簡略化されたCXG呼び出しとして
/// 表現するが、正式なUNITWRITE呼び出し規約は再現しない」という方針と
/// 同じ種類の意図的な簡略化）。
///
/// この結果、`STRING[n]`変数1個が占めるワード数は`1 + n`
/// （[`crate::codegen::CodeGenerator`]の`declare_vars`参照）。
pub const BUILTIN_WRITELN_STRVAR: u8 = 5;
