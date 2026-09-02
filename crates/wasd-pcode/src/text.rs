//! p-codeのテキスト表現（逆アセンブリ的なニーモニック表示）。
//!
//! バイナリ形式（実際のバイト列）は今回のスコープ外（`crates/wasd-pcode/src/lib.rs`
//! のドキュメント参照）。オペコード番号・オペランドのバイトエンコーディングが
//! 未確認である以上、確認前に架空のバイト列を出力することは誤解を招くため、
//! ここではIRをそのまま読める形にするテキスト表示のみを提供する。

use std::fmt;

use crate::ir::{Instruction, PCodeModule};
use crate::opcode::{ConfirmedOp, Opcode, UnconfirmedOp};

impl fmt::Display for PCodeModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, instruction) in self.instructions.iter().enumerate() {
            writeln!(f, "{index:>5}: {}", format_instruction(instruction))?;
        }
        if !self.routines.is_empty() {
            writeln!(f, "; routines (entry, params, data_size, is_func):")?;
            for r in &self.routines {
                writeln!(
                    f,
                    ";   entry={} params={} data_size={} is_func={}",
                    r.entry.0, r.param_count, r.data_size, r.is_func
                )?;
            }
        }
        if !self.string_pool.is_empty() {
            writeln!(f, "; string pool:")?;
            for (index, value) in self.string_pool.iter().enumerate() {
                writeln!(f, ";   [{index}] {value:?}")?;
            }
        }
        Ok(())
    }
}

fn format_instruction(instruction: &Instruction) -> String {
    format_opcode(&instruction.opcode)
}

fn format_opcode(opcode: &Opcode) -> String {
    match opcode {
        Opcode::Confirmed(op) => format_confirmed(op),
        Opcode::Unconfirmed(op) => format_unconfirmed(op),
    }
}

fn format_confirmed(op: &ConfirmedOp) -> String {
    match op {
        ConfirmedOp::Cpl(target) => format!("CPL {}", target.0),
        ConfirmedOp::Cpg(target) => format!("CPG {}", target.0),
        ConfirmedOp::Cpi(db, target) => format!("CPI {db},{}", target.0),
        ConfirmedOp::Scpi1(target) => format!("SCPI1 {}", target.0),
        ConfirmedOp::Scpi2(target) => format!("SCPI2 {}", target.0),
        ConfirmedOp::Rpu(b) => format!("RPU {b}"),
        ConfirmedOp::Cxg(seg, proc) => format!("CXG {seg},{proc}"),
    }
}

fn format_unconfirmed(op: &UnconfirmedOp) -> String {
    match op {
        UnconfirmedOp::Ldc(value) => format!("LDC {value}"),
        UnconfirmedOp::Lod(level, addr) => format!("LOD {},{}", level.0, addr.0),
        UnconfirmedOp::Str(level, addr) => format!("STR {},{}", level.0, addr.0),
        UnconfirmedOp::Lda(level, addr) => format!("LDA {},{}", level.0, addr.0),
        UnconfirmedOp::Ind => "IND".to_string(),
        UnconfirmedOp::Sti => "STI".to_string(),
        UnconfirmedOp::Adi => "ADI".to_string(),
        UnconfirmedOp::Sbi => "SBI".to_string(),
        UnconfirmedOp::Mpi => "MPI".to_string(),
        UnconfirmedOp::Dvi => "DVI".to_string(),
        UnconfirmedOp::Mod => "MOD".to_string(),
        UnconfirmedOp::Ngi => "NGI".to_string(),
        UnconfirmedOp::Equ => "EQU".to_string(),
        UnconfirmedOp::Neq => "NEQ".to_string(),
        UnconfirmedOp::Leq => "LEQ".to_string(),
        UnconfirmedOp::Geq => "GEQ".to_string(),
        UnconfirmedOp::And => "AND".to_string(),
        UnconfirmedOp::Ior => "IOR".to_string(),
        UnconfirmedOp::Not => "NOT".to_string(),
        UnconfirmedOp::Ujp(target) => format!("UJP {}", target.0),
        UnconfirmedOp::Fjp(target) => format!("FJP {}", target.0),
        UnconfirmedOp::Stp => "STP".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use wasd_ast::Span;

    use super::*;
    use crate::opcode::{Address, CodeAddress, Level};

    fn instr(opcode: UnconfirmedOp) -> Instruction {
        Instruction {
            opcode: Opcode::Unconfirmed(opcode),
            span: Span::new(0, 1),
        }
    }

    fn confirmed_instr(opcode: ConfirmedOp) -> Instruction {
        Instruction {
            opcode: Opcode::Confirmed(opcode),
            span: Span::new(0, 1),
        }
    }

    #[test]
    fn renders_one_instruction_per_line_with_an_index_prefix() {
        let module = PCodeModule {
            instructions: vec![
                instr(UnconfirmedOp::Ldc(1)),
                instr(UnconfirmedOp::Ldc(2)),
                instr(UnconfirmedOp::Adi),
                instr(UnconfirmedOp::Str(Level(0), Address(0))),
                instr(UnconfirmedOp::Stp),
            ],
            global_data_words: 1,
            routines: Vec::new(),
            entry: CodeAddress(0),
            string_pool: Vec::new(),
        };

        let text = module.to_string();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(
            lines,
            vec![
                "    0: LDC 1",
                "    1: LDC 2",
                "    2: ADI",
                "    3: STR 0,0",
                "    4: STP",
            ]
        );
    }

    #[test]
    fn renders_jump_targets_as_instruction_indices() {
        let module = PCodeModule {
            instructions: vec![
                instr(UnconfirmedOp::Fjp(CodeAddress(2))),
                instr(UnconfirmedOp::Ujp(CodeAddress(0))),
            ],
            global_data_words: 0,
            routines: Vec::new(),
            entry: CodeAddress(0),
            string_pool: Vec::new(),
        };

        let text = module.to_string();
        assert!(text.contains("FJP 2"));
        assert!(text.contains("UJP 0"));
    }

    #[test]
    fn renders_call_and_return_instructions() {
        let module = PCodeModule {
            instructions: vec![
                confirmed_instr(ConfirmedOp::Cpg(CodeAddress(3))),
                confirmed_instr(ConfirmedOp::Rpu(2)),
            ],
            global_data_words: 0,
            routines: Vec::new(),
            entry: CodeAddress(0),
            string_pool: Vec::new(),
        };

        let text = module.to_string();
        assert!(text.contains("CPG 3"));
        assert!(text.contains("RPU 2"));
    }
}
