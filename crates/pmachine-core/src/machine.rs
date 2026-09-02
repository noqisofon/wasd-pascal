//! [`PMachine`]本体: フェッチ・デコード・実行ループとオペコードの実装。
//! 設計上の判断（メモリモデル・呼び出し規約の物理的な組み立て方）は
//! `crates/pmachine-core/src/lib.rs`のモジュールドキュメント参照。

use std::collections::HashMap;

use wasd_pcode::{Address, CodeAddress, ConfirmedOp, Level, Opcode, PCodeModule, UnconfirmedOp};

use crate::error::RuntimeError;

/// 呼び出しによってスタック上に積まれた1つの活性化レコードについて、
/// アドレッシングと`RPU`での復帰に必要な最小限の情報。
///
/// マーク・スタック制御ワード（MSCW）の実データそのものは`stack`上には
/// 確保しない（`crates/pmachine-core/src/lib.rs`のモジュールドキュメント
/// 「メモリモデル」参照）。この構造体がMSCWの代わりを果たす。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRecord {
    /// このフレームにおいて`Address(5)`が指す`stack`上の実位置。
    /// `Address(a)`（`a >= 5`）は`stack[base + (a - 5)]`に対応する。
    pub base: usize,
    /// 静的リンク（`MSSTAT`相当）。`frames`中のインデックス。`None`は
    /// 「これより上（グローバルデータ領域）」を意味する。
    pub static_link: Option<usize>,
    /// 動的リンク（`MSDYN`相当）。呼び出し元が実行していたフレーム。
    /// `None`は「呼び出し元はグローバルスコープ（`PROGRAM`本体）だった」
    /// ことを意味する。
    pub dynamic_link: Option<usize>,
    /// 復帰先IPC（`MSIPC`相当）。呼び出し命令の直後の命令インデックス。
    pub return_ipc: usize,
}

/// レベル差アドレッシングの解決結果: グローバルデータ領域を指すか、
/// 特定の活性化レコードを指すか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Storage {
    Global,
    Frame(usize),
}

/// p-machineの実行状態。
///
/// タスク依頼が示した構造体案とは、以下の点で意図的に異なる（理由は
/// `crates/pmachine-core/src/lib.rs`のモジュールドキュメント参照）:
/// - `mp`は`stack`中の生アドレスではなく、[`FrameRecord`]の列
///   （`frames`）へのインデックス（`Option<usize>`、`None`はグローバル
///   スコープを実行中であることを表す）。
/// - `base`（BASEレジスタ、グローバルデータ領域の先頭）は本クレームでは
///   常に`0`固定であり、専用フィールドは持たない。
/// - `curproc`は、実機の「プロシージャ辞書中のインデックス」という
///   意味論を再現できていない（本IRは呼び出し先を直接`CodeAddress`として
///   保持する簡略化を採用しているため。`wasd_pcode::ConfirmedOp::Cpl`の
///   ドキュメント参照）。現在実行中のルーチンのエントリアドレス下位
///   8ビットという、デバッグ用の弱い近似値を保持するのみ。
pub struct PMachine {
    /// グローバルデータ領域（`0..global_data_words`、常駐）+ その上に
    /// 積まれる活性化レコード・式評価用の一時値。
    stack: Vec<i16>,
    /// 呼び出し中の活性化レコードのスタック。`frames.last()`が
    /// （存在すれば）現在のフレーム。
    frames: Vec<FrameRecord>,
    /// 現在の活性化レコード。`None`は`PROGRAM`本体（グローバルスコープ）
    /// を直接実行中であることを表す。
    mp: Option<usize>,
    /// 次に実行する命令の、`code`中のインデックス。
    ipc: usize,
    /// デバッグ用の弱い近似値（構造体ドキュメント参照）。
    curproc: u8,
    /// `wasd-pcode`が生成した命令列。
    code: Vec<wasd_pcode::Instruction>,
    /// 呼び出し先エントリアドレス（[`CodeAddress::0`]）→ルーチン
    /// メタデータ。実機のプロシージャ辞書に相当
    /// （`wasd_pcode::RoutineMeta`のドキュメント参照）。
    routine_table: HashMap<u32, wasd_pcode::RoutineMeta>,
    /// グローバルデータ領域のワード数（`stack[0..global_data_words]`）。
    global_data_words: u16,
    halted: bool,
}

impl PMachine {
    /// `wasd-pcode`が生成した[`PCodeModule`]を読み込み、実行準備の
    /// 整った状態を作る。グローバルデータ領域はゼロ初期化される。
    pub fn new(module: PCodeModule) -> Self {
        let PCodeModule {
            instructions,
            global_data_words,
            routines,
            entry,
        } = module;
        let routine_table = routines.into_iter().map(|r| (r.entry.0, r)).collect();
        Self {
            stack: vec![0; global_data_words as usize],
            frames: Vec::new(),
            mp: None,
            // `PROGRAM`本体はPROCEDURE/FUNCTION本体より後に生成される
            // ため、命令列インデックス0から始めてはならない
            // （`wasd_pcode::PCodeModule::entry`のドキュメント参照）。
            ipc: entry.0 as usize,
            curproc: 0,
            code: instructions,
            routine_table,
            global_data_words,
            halted: false,
        }
    }

    /// 実行が`STP`等によって終了しているか。
    pub fn is_halted(&self) -> bool {
        self.halted
    }

    /// 現在のSP（スタックの高さ、`stack`の要素数）。呼び出し前後の
    /// スタックバランスの検証（`RPU`の`b`パラメータの検証。
    /// `crates/pmachine-core/src/lib.rs`のモジュールドキュメント参照）に使う。
    pub fn sp(&self) -> usize {
        self.stack.len()
    }

    /// 現在のIPC。
    pub fn ipc(&self) -> usize {
        self.ipc
    }

    /// 現在の呼び出しの深さ（アクティブな活性化レコードの数）。
    pub fn call_depth(&self) -> usize {
        self.frames.len()
    }

    /// グローバル変数1件の現在値を読む（デバッグ・テスト用）。
    pub fn global(&self, addr: u16) -> Option<i16> {
        if addr >= self.global_data_words {
            return None;
        }
        self.stack.get(addr as usize).copied()
    }

    /// グローバルデータ領域全体のスナップショット（デバッグ・テスト用、
    /// `wasdc run`の出力にも使う）。
    pub fn globals(&self) -> &[i16] {
        &self.stack[..self.global_data_words as usize]
    }

    /// haltするまで実行する。
    pub fn run(&mut self) -> Result<(), RuntimeError> {
        while !self.halted {
            self.step()?;
        }
        Ok(())
    }

    /// 1命令実行する。既にhaltしている場合は何もしない。
    pub fn step(&mut self) -> Result<(), RuntimeError> {
        if self.halted {
            return Ok(());
        }
        let instr = *self
            .code
            .get(self.ipc)
            .ok_or(RuntimeError::IpcOutOfBounds)?;
        // 分岐・呼び出し・復帰命令は、この「次の命令」への既定の前進を
        // 必要に応じて上書きする（実際のIPC更新は各命令のハンドラが行う）。
        self.ipc += 1;
        self.execute(instr.opcode)
    }

    fn execute(&mut self, opcode: Opcode) -> Result<(), RuntimeError> {
        match opcode {
            Opcode::Confirmed(op) => self.execute_confirmed(op),
            Opcode::Unconfirmed(op) => self.execute_unconfirmed(op),
        }
    }

    // ---- 呼び出し/復帰 ----

    fn execute_confirmed(&mut self, op: ConfirmedOp) -> Result<(), RuntimeError> {
        match op {
            // CPL: 直接の子（現在のフレームが静的な親になる）。
            ConfirmedOp::Cpl(target) => {
                let static_link = self.mp;
                self.call(target, static_link)
            }
            // CPG: 常にBASE（グローバル）が静的な親。
            ConfirmedOp::Cpg(target) => self.call(target, None),
            ConfirmedOp::Cpi(db, target) => {
                let static_link = self.walk_static(self.mp, db as u16)?;
                self.call(target, static_link)
            }
            ConfirmedOp::Scpi1(target) => {
                let static_link = self.walk_static(self.mp, 1)?;
                self.call(target, static_link)
            }
            ConfirmedOp::Scpi2(target) => {
                let static_link = self.walk_static(self.mp, 2)?;
                self.call(target, static_link)
            }
            ConfirmedOp::Rpu(b) => self.exec_rpu(b),
        }
    }

    /// 現在のフレームから静的リンクを`hops`回辿った先のフレームを返す。
    /// 辿りきる前にグローバル（`None`）へ到達したら、その時点で`None`を
    /// 返す（本クレートのスコープでは最大ネスト深さ2なので、これ以上
    /// 深く辿る状況は現状発生しないが、`CPI`/`SCPI2`向けに一般化してある）。
    fn walk_static(&self, start: Option<usize>, hops: u16) -> Result<Option<usize>, RuntimeError> {
        let mut cur = start;
        let mut remaining = hops;
        while remaining > 0 {
            match cur {
                Some(frame) => {
                    cur = self.frames[frame].static_link;
                    remaining -= 1;
                }
                None => return Ok(None),
            }
        }
        Ok(cur)
    }

    /// `CPL`/`CPG`/`CPI`/`SCPI1`/`SCPI2`共通の呼び出し処理。
    /// 呼び出し規約の詳細は`crates/pmachine-core/src/lib.rs`のモジュール
    /// ドキュメント「呼び出し規約: パラメータの物理的な並べ替えについて」
    /// 参照。
    fn call(
        &mut self,
        target: CodeAddress,
        static_link: Option<usize>,
    ) -> Result<(), RuntimeError> {
        let meta = *self
            .routine_table
            .get(&target.0)
            .ok_or(RuntimeError::UnknownRoutine)?;
        let param_count = meta.param_count as usize;

        if self.stack.len() < param_count {
            return Err(RuntimeError::StackUnderflow);
        }
        let args_start = self.stack.len() - param_count;
        let args: Vec<i16> = self.stack.split_off(args_start);

        // `base`はアドレス5が指す位置。呼び出し元が既に積んでいた
        // パラメータの直後（=それらを一旦取り除いた今のスタック高さ）。
        let base = self.stack.len();
        self.stack
            .resize(self.stack.len() + meta.data_size as usize, 0);
        self.stack.extend_from_slice(&args);
        if meta.is_func {
            self.stack.push(0);
        }

        self.frames.push(FrameRecord {
            base,
            static_link,
            dynamic_link: self.mp,
            return_ipc: self.ipc,
        });
        self.mp = Some(self.frames.len() - 1);
        self.curproc = target.0 as u8;
        self.ipc = target.0 as usize;
        Ok(())
    }

    /// `RPU <b>`: 呼び出し元の状態を復元し、活性化レコードの先頭
    /// （`base`）から`b`ワード分を切り詰める。それより上に残っていた分
    /// （`FUNCTION`の戻り値、存在すれば1ワード）はそのまま生き残る。
    fn exec_rpu(&mut self, b: u16) -> Result<(), RuntimeError> {
        if self.mp.is_none() {
            return Err(RuntimeError::NoActiveFrame);
        }
        debug_assert_eq!(self.mp, Some(self.frames.len() - 1));
        let frame = self
            .frames
            .pop()
            .expect("mp indicates an active frame but the frame stack is empty");

        let remove_end = frame
            .base
            .checked_add(b as usize)
            .ok_or(RuntimeError::AddressOutOfRange)?;
        if remove_end > self.stack.len() {
            return Err(RuntimeError::StackUnderflow);
        }
        let leftover: Vec<i16> = self.stack.split_off(remove_end);
        self.stack.truncate(frame.base);
        self.stack.extend_from_slice(&leftover);

        self.mp = frame.dynamic_link;
        self.ipc = frame.return_ipc;
        Ok(())
    }

    // ---- アドレッシング ----

    /// `level`個の静的リンクを辿った先が、グローバルデータ領域か特定の
    /// 活性化レコードかを解決する。
    fn resolve(&self, level: Level) -> Result<Storage, RuntimeError> {
        match self.mp {
            None => {
                if level.0 == 0 {
                    Ok(Storage::Global)
                } else {
                    Err(RuntimeError::NoActiveFrame)
                }
            }
            Some(start) => {
                let mut cur = start;
                let mut hops = level.0;
                loop {
                    if hops == 0 {
                        return Ok(Storage::Frame(cur));
                    }
                    match self.frames[cur].static_link {
                        Some(parent) => {
                            cur = parent;
                            hops -= 1;
                        }
                        None => return Ok(Storage::Global),
                    }
                }
            }
        }
    }

    /// 解決済みの[`Storage`]と`Address`から、`stack`中の実インデックスを
    /// 求める。
    fn addr_index(&self, storage: Storage, addr: Address) -> Result<usize, RuntimeError> {
        match storage {
            Storage::Global => Ok(addr.0 as usize),
            Storage::Frame(f) => {
                let offset = (addr.0 as usize)
                    .checked_sub(5)
                    .ok_or(RuntimeError::AddressOutOfRange)?;
                Ok(self.frames[f].base + offset)
            }
        }
    }

    fn read_slot(&self, level: Level, addr: Address) -> Result<i16, RuntimeError> {
        let storage = self.resolve(level)?;
        let idx = self.addr_index(storage, addr)?;
        self.stack
            .get(idx)
            .copied()
            .ok_or(RuntimeError::AddressOutOfRange)
    }

    fn write_slot(&mut self, level: Level, addr: Address, value: i16) -> Result<(), RuntimeError> {
        let storage = self.resolve(level)?;
        let idx = self.addr_index(storage, addr)?;
        let slot = self
            .stack
            .get_mut(idx)
            .ok_or(RuntimeError::AddressOutOfRange)?;
        *slot = value;
        Ok(())
    }

    /// `LDA`: 解決済みアドレスを1ワード（`i16`）の値として求める
    /// （`IND`/`STI`で使う「アドレスそのものをスタックへ積む」ため）。
    fn addr_as_word(&self, level: Level, addr: Address) -> Result<i16, RuntimeError> {
        let storage = self.resolve(level)?;
        let idx = self.addr_index(storage, addr)?;
        i16::try_from(idx).map_err(|_| RuntimeError::AddressOutOfRange)
    }

    // ---- スタック操作の共通ヘルパー ----

    fn push(&mut self, value: i16) {
        self.stack.push(value);
    }

    fn pop(&mut self) -> Result<i16, RuntimeError> {
        self.stack.pop().ok_or(RuntimeError::StackUnderflow)
    }

    fn pop_bool(&mut self) -> Result<bool, RuntimeError> {
        Ok(self.pop()? != 0)
    }

    fn push_bool(&mut self, value: bool) {
        self.push(if value { 1 } else { 0 });
    }

    // ---- 未確認オペコード群の実行 ----

    fn execute_unconfirmed(&mut self, op: UnconfirmedOp) -> Result<(), RuntimeError> {
        match op {
            UnconfirmedOp::Ldc(value) => {
                self.push(value);
                Ok(())
            }
            UnconfirmedOp::Lod(level, addr) => {
                let value = self.read_slot(level, addr)?;
                self.push(value);
                Ok(())
            }
            UnconfirmedOp::Str(level, addr) => {
                let value = self.pop()?;
                self.write_slot(level, addr, value)
            }
            UnconfirmedOp::Lda(level, addr) => {
                let word = self.addr_as_word(level, addr)?;
                self.push(word);
                Ok(())
            }
            UnconfirmedOp::Ind => {
                let addr = self.pop()?;
                let idx = usize::try_from(addr).map_err(|_| RuntimeError::AddressOutOfRange)?;
                let value = *self.stack.get(idx).ok_or(RuntimeError::AddressOutOfRange)?;
                self.push(value);
                Ok(())
            }
            UnconfirmedOp::Sti => {
                let value = self.pop()?;
                let addr = self.pop()?;
                let idx = usize::try_from(addr).map_err(|_| RuntimeError::AddressOutOfRange)?;
                let slot = self
                    .stack
                    .get_mut(idx)
                    .ok_or(RuntimeError::AddressOutOfRange)?;
                *slot = value;
                Ok(())
            }
            UnconfirmedOp::Adi => self.binop_int(i16::wrapping_add),
            UnconfirmedOp::Sbi => self.binop_int(i16::wrapping_sub),
            UnconfirmedOp::Mpi => self.binop_int(i16::wrapping_mul),
            UnconfirmedOp::Dvi => {
                let b = self.pop()?;
                let a = self.pop()?;
                if b == 0 {
                    return Err(RuntimeError::DivisionByZero);
                }
                self.push(a.wrapping_div(b));
                Ok(())
            }
            UnconfirmedOp::Mod => {
                let b = self.pop()?;
                let a = self.pop()?;
                if b == 0 {
                    return Err(RuntimeError::DivisionByZero);
                }
                self.push(a.wrapping_rem(b));
                Ok(())
            }
            UnconfirmedOp::Ngi => {
                let a = self.pop()?;
                self.push(a.wrapping_neg());
                Ok(())
            }
            UnconfirmedOp::Equ => self.cmp(|a, b| a == b),
            UnconfirmedOp::Neq => self.cmp(|a, b| a != b),
            UnconfirmedOp::Les => self.cmp(|a, b| a < b),
            UnconfirmedOp::Leq => self.cmp(|a, b| a <= b),
            UnconfirmedOp::Grt => self.cmp(|a, b| a > b),
            UnconfirmedOp::Geq => self.cmp(|a, b| a >= b),
            UnconfirmedOp::And => {
                let b = self.pop_bool()?;
                let a = self.pop_bool()?;
                self.push_bool(a && b);
                Ok(())
            }
            UnconfirmedOp::Ior => {
                let b = self.pop_bool()?;
                let a = self.pop_bool()?;
                self.push_bool(a || b);
                Ok(())
            }
            UnconfirmedOp::Not => {
                let a = self.pop_bool()?;
                self.push_bool(!a);
                Ok(())
            }
            UnconfirmedOp::Ujp(target) => {
                self.ipc = target.0 as usize;
                Ok(())
            }
            UnconfirmedOp::Fjp(target) => {
                let cond = self.pop_bool()?;
                if !cond {
                    self.ipc = target.0 as usize;
                }
                Ok(())
            }
            UnconfirmedOp::Stp => {
                self.halted = true;
                Ok(())
            }
        }
    }

    fn binop_int(&mut self, f: impl FnOnce(i16, i16) -> i16) -> Result<(), RuntimeError> {
        let b = self.pop()?;
        let a = self.pop()?;
        self.push(f(a, b));
        Ok(())
    }

    fn cmp(&mut self, f: impl FnOnce(i16, i16) -> bool) -> Result<(), RuntimeError> {
        let b = self.pop()?;
        let a = self.pop()?;
        self.push_bool(f(a, b));
        Ok(())
    }
}
