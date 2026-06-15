//! Instruction selection: Low IR → target machine instructions with virtual registers.

use crate::low_ir::{CmpOp, LowFunction, LowOp, VReg};
use crate::CodegenError;
use arc_targets::TargetDescription;

/// A machine instruction with virtual registers.
#[derive(Debug, Clone)]
pub enum MachInst {
    /// mov dst, imm64
    MovImm64 { dst: Operand, value: i64 },
    /// add dst, src (dst += src, two-address)
    Add { dst: Operand, src: Operand },
    /// sub dst, src (dst -= src)
    Sub { dst: Operand, src: Operand },
    /// imul dst, src
    IMul { dst: Operand, src: Operand },
    /// idiv src (rax = rax / src, rdx = rax % src)
    /// Requires dividend in rax, clobbers rdx.
    IDiv { divisor: Operand },
    /// cmp lhs, rhs
    Cmp { lhs: Operand, rhs: Operand },
    /// Set byte based on condition code (setcc dst).
    SetCC { dst: Operand, cc: CondCode },
    /// movzx dst, src (zero-extend byte to 64-bit)
    Movzx { dst: Operand, src: Operand },
    /// mov dst, src
    Mov { dst: Operand, src: Operand },
    /// jmp to block index
    Jmp { target: usize },
    /// Conditional jump: test cond, cond; jnz target
    TestAndJnz {
        cond: Operand,
        true_target: usize,
        false_target: usize,
    },
    /// call label
    Call {
        callee: String,
        args: Vec<Operand>,
        dst: Option<Operand>,
    },
    /// ret
    Ret { value: Option<Operand> },
    /// Sign-extend rax to rdx:rax (cqo). Used before idiv.
    Cqo,
    /// push reg
    Push { src: Operand },
    /// pop reg
    Pop { dst: Operand },
    /// Load from stack frame: mov dst, [rbp + offset].
    /// Used for stack-passed arguments beyond the register file.
    LoadStack { dst: Operand, offset: i32 },
    /// Label marker (no actual instruction emitted, used for block starts)
    Label { index: usize },
    /// `sub rsp, size; lea dst, [rsp]`: allocate stack space and get pointer.
    StackAlloc { dst: Operand, size: i32 },
    /// `mov dst, [addr]`: load 64-bit from memory address in register.
    LoadMem { dst: Operand, addr: Operand },
    /// `mov [addr], src`: store 64-bit to memory address in register.
    StoreMem { addr: Operand, src: Operand },
    /// `mov dst, [base + index*8]`: load element with scaled index.
    LoadElemMem {
        dst: Operand,
        base: Operand,
        index: Operand,
    },
}

/// An instruction operand — either a virtual register or physical register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operand {
    VReg(VReg),
    PReg(u8),
}

impl Operand {
    pub fn is_vreg(self) -> bool {
        matches!(self, Operand::VReg(_))
    }

    pub fn vreg(self) -> Option<VReg> {
        match self {
            Operand::VReg(v) => Some(v),
            Operand::PReg(_) => None,
        }
    }
}

/// x86_64 condition codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondCode {
    E,  // equal
    Ne, // not equal
    L,  // less (signed)
    Le, // less or equal
    G,  // greater (signed)
    Ge, // greater or equal
}

impl From<CmpOp> for CondCode {
    fn from(op: CmpOp) -> Self {
        match op {
            CmpOp::Eq => CondCode::E,
            CmpOp::Ne => CondCode::Ne,
            CmpOp::Slt => CondCode::L,
            CmpOp::Sle => CondCode::Le,
            CmpOp::Sgt => CondCode::G,
            CmpOp::Sge => CondCode::Ge,
        }
    }
}

/// A machine-level function after instruction selection.
#[derive(Debug, Clone)]
pub struct MachFunction {
    pub name: String,
    pub blocks: Vec<MachBlock>,
    pub vreg_count: u32,
    /// Virtual registers that are used for function parameters.
    pub param_vregs: Vec<VReg>,
    pub has_return: bool,
}

/// A machine-level basic block.
#[derive(Debug, Clone)]
pub struct MachBlock {
    pub label: String,
    pub insts: Vec<MachInst>,
}

/// Select x86_64 instructions for a low-IR function.
pub fn select_instructions(
    func: &LowFunction,
    _target: &TargetDescription,
) -> Result<MachFunction, CodegenError> {
    let mut blocks = Vec::new();
    let param_vregs: Vec<VReg> = func.params.iter().map(|(v, _)| *v).collect();

    for (block_idx, block) in func.blocks.iter().enumerate() {
        let mut insts = Vec::new();
        insts.push(MachInst::Label { index: block_idx });

        for op in &block.ops {
            match op {
                LowOp::LoadImm { dst, value } => {
                    insts.push(MachInst::MovImm64 {
                        dst: Operand::VReg(*dst),
                        value: *value,
                    });
                }
                LowOp::Add { dst, lhs, rhs } => {
                    // x86 add is two-address: dst += src
                    // mov dst, lhs; add dst, rhs
                    insts.push(MachInst::Mov {
                        dst: Operand::VReg(*dst),
                        src: Operand::VReg(*lhs),
                    });
                    insts.push(MachInst::Add {
                        dst: Operand::VReg(*dst),
                        src: Operand::VReg(*rhs),
                    });
                }
                LowOp::Sub { dst, lhs, rhs } => {
                    insts.push(MachInst::Mov {
                        dst: Operand::VReg(*dst),
                        src: Operand::VReg(*lhs),
                    });
                    insts.push(MachInst::Sub {
                        dst: Operand::VReg(*dst),
                        src: Operand::VReg(*rhs),
                    });
                }
                LowOp::Mul { dst, lhs, rhs } => {
                    // imul dst, src is three-address in some forms,
                    // but we use two-address: mov dst, lhs; imul dst, rhs
                    insts.push(MachInst::Mov {
                        dst: Operand::VReg(*dst),
                        src: Operand::VReg(*lhs),
                    });
                    insts.push(MachInst::IMul {
                        dst: Operand::VReg(*dst),
                        src: Operand::VReg(*rhs),
                    });
                }
                LowOp::Div { dst, lhs, rhs } => {
                    // idiv uses rax for dividend, result in rax
                    use arc_targets::x86_64::{RAX, RDX};
                    insts.push(MachInst::Mov {
                        dst: Operand::PReg(RAX),
                        src: Operand::VReg(*lhs),
                    });
                    insts.push(MachInst::Cqo);
                    insts.push(MachInst::IDiv {
                        divisor: Operand::VReg(*rhs),
                    });
                    insts.push(MachInst::Mov {
                        dst: Operand::VReg(*dst),
                        src: Operand::PReg(RAX),
                    });
                    // RDX is clobbered (contains remainder)
                    let _ = RDX;
                }
                LowOp::Cmp {
                    dst,
                    op: cmp_op,
                    lhs,
                    rhs,
                } => {
                    insts.push(MachInst::Cmp {
                        lhs: Operand::VReg(*lhs),
                        rhs: Operand::VReg(*rhs),
                    });
                    insts.push(MachInst::SetCC {
                        dst: Operand::VReg(*dst),
                        cc: CondCode::from(*cmp_op),
                    });
                    insts.push(MachInst::Movzx {
                        dst: Operand::VReg(*dst),
                        src: Operand::VReg(*dst),
                    });
                }
                LowOp::Copy { dst, src } => {
                    insts.push(MachInst::Mov {
                        dst: Operand::VReg(*dst),
                        src: Operand::VReg(*src),
                    });
                }
                LowOp::Jump { target } => {
                    insts.push(MachInst::Jmp { target: *target });
                }
                LowOp::CondJump {
                    cond,
                    true_target,
                    false_target,
                } => {
                    insts.push(MachInst::TestAndJnz {
                        cond: Operand::VReg(*cond),
                        true_target: *true_target,
                        false_target: *false_target,
                    });
                }
                LowOp::Call { dst, callee, args } => {
                    let arg_ops: Vec<Operand> = args.iter().map(|v| Operand::VReg(*v)).collect();
                    let dst_op = dst.map(Operand::VReg);
                    insts.push(MachInst::Call {
                        callee: callee.clone(),
                        args: arg_ops,
                        dst: dst_op,
                    });
                }
                LowOp::Ret { value } => {
                    insts.push(MachInst::Ret {
                        value: value.map(Operand::VReg),
                    });
                }
                LowOp::StackAlloc { dst, size } => {
                    insts.push(MachInst::StackAlloc {
                        dst: Operand::VReg(*dst),
                        size: *size as i32,
                    });
                }
                LowOp::MemLoad { dst, addr } => {
                    insts.push(MachInst::LoadMem {
                        dst: Operand::VReg(*dst),
                        addr: Operand::VReg(*addr),
                    });
                }
                LowOp::MemStore { addr, src } => {
                    insts.push(MachInst::StoreMem {
                        addr: Operand::VReg(*addr),
                        src: Operand::VReg(*src),
                    });
                }
                LowOp::MemLoadElem { dst, base, index } => {
                    insts.push(MachInst::LoadElemMem {
                        dst: Operand::VReg(*dst),
                        base: Operand::VReg(*base),
                        index: Operand::VReg(*index),
                    });
                }
            }
        }

        blocks.push(MachBlock {
            label: block.label.clone(),
            insts,
        });
    }

    Ok(MachFunction {
        name: func.name.clone(),
        blocks,
        vreg_count: func.vreg_count(),
        param_vregs,
        has_return: func.result_type.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::low_ir::*;
    use arc_targets::x86_64;

    #[test]
    fn select_add_instructions() {
        let func = LowFunction {
            name: "add".to_string(),
            params: vec![(VReg(0), MachType::I64), (VReg(1), MachType::I64)],
            result_type: Some(MachType::I64),
            blocks: vec![LowBlock {
                label: "entry".to_string(),
                params: vec![],
                ops: vec![
                    LowOp::Add {
                        dst: VReg(2),
                        lhs: VReg(0),
                        rhs: VReg(1),
                    },
                    LowOp::Ret {
                        value: Some(VReg(2)),
                    },
                ],
            }],
            next_vreg: 3,
        };

        let target = x86_64::target();
        let mfunc = select_instructions(&func, &target).unwrap();
        assert_eq!(mfunc.name, "add");
        assert_eq!(mfunc.blocks.len(), 1);

        // Should be: Label, Mov(v2, v0), Add(v2, v1), Ret(v2)
        let insts = &mfunc.blocks[0].insts;
        assert!(matches!(insts[0], MachInst::Label { .. }));
        assert!(matches!(insts[1], MachInst::Mov { .. }));
        assert!(matches!(insts[2], MachInst::Add { .. }));
        assert!(matches!(insts[3], MachInst::Ret { .. }));
    }
}
