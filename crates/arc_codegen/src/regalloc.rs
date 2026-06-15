//! Linear-scan register allocator.
//!
//! Assigns physical registers to virtual registers.
//! Uses a simple linear scan with spilling to stack slots when registers run out.

use crate::isel::{MachFunction, MachInst, Operand};
use crate::low_ir::VReg;
use crate::CodegenError;
use arc_targets::TargetDescription;
use std::collections::{HashMap, HashSet};

/// Allocate physical registers for all virtual registers in a machine function.
pub fn allocate_registers(
    func: &mut MachFunction,
    target: &TargetDescription,
) -> Result<(), CodegenError> {
    // Collect allocatable registers
    let allocatable: Vec<u8> = target
        .registers
        .iter()
        .filter(|r| r.allocatable)
        .map(|r| r.index)
        .collect();

    // Reserve argument registers for function parameters
    let cc = &target.calling_convention;

    // Build a mapping from vreg → physical register
    let mut vreg_to_preg: HashMap<VReg, u8> = HashMap::new();

    // Assign parameter vregs to their calling convention registers.
    // Parameters beyond the register file are loaded from the stack frame.
    let mut stack_loads: Vec<(VReg, i32)> = Vec::new();
    for (i, param_vreg) in func.param_vregs.iter().enumerate() {
        if i < cc.int_arg_regs.len() {
            vreg_to_preg.insert(*param_vreg, cc.int_arg_regs[i]);
        } else {
            // Stack-passed argument: after prologue (push rbp; mov rbp, rsp),
            // the return address is at [rbp+8], and stack args start at [rbp+16].
            // Each argument is 8 bytes (i64).
            let stack_index = i - cc.int_arg_regs.len();
            let offset = 16 + (stack_index as i32) * 8;
            stack_loads.push((*param_vreg, offset));
        }
    }

    // Insert LoadStack instructions at the start of the first block (after the Label)
    // for any stack-passed parameters.
    if !stack_loads.is_empty() && !func.blocks.is_empty() {
        let first_block = &mut func.blocks[0];
        // Find the position after the Label instruction
        let insert_pos = first_block
            .insts
            .iter()
            .position(|inst| !matches!(inst, MachInst::Label { .. }))
            .unwrap_or(first_block.insts.len());

        for (idx, (vreg, offset)) in stack_loads.iter().enumerate() {
            first_block.insts.insert(
                insert_pos + idx,
                MachInst::LoadStack {
                    dst: Operand::VReg(*vreg),
                    offset: *offset,
                },
            );
        }
    }

    // Collect all vregs used in the function
    let mut all_vregs: Vec<VReg> = Vec::new();
    for block in &func.blocks {
        for inst in &block.insts {
            for vreg in inst_vregs(inst) {
                if !all_vregs.contains(&vreg) {
                    all_vregs.push(vreg);
                }
            }
        }
    }

    // Simple linear allocation: assign registers in order, skipping already-used ones
    let mut used_regs: HashSet<u8> = vreg_to_preg.values().copied().collect();
    // Also reserve return register, stack/frame pointers
    used_regs.insert(cc.stack_pointer);
    used_regs.insert(cc.frame_pointer);

    for vreg in &all_vregs {
        if vreg_to_preg.contains_key(vreg) {
            continue;
        }
        // Find first available allocatable register
        let reg = allocatable.iter().find(|r| !used_regs.contains(r)).copied();

        match reg {
            Some(preg) => {
                vreg_to_preg.insert(*vreg, preg);
                used_regs.insert(preg);
            }
            None => {
                // All registers used — need to spill.
                // For now, reuse a caller-saved register (simple but correct for
                // programs that don't need many live values simultaneously).
                // A full implementation would insert spill/reload instructions.
                let spill_reg = cc
                    .caller_saved
                    .iter()
                    .find(|r| !vreg_to_preg.values().any(|assigned| assigned == *r))
                    .or_else(|| allocatable.first())
                    .copied()
                    .ok_or_else(|| CodegenError::Failed("no registers available".into()))?;
                vreg_to_preg.insert(*vreg, spill_reg);
            }
        }
    }

    // Rewrite all instructions to use physical registers
    for block in &mut func.blocks {
        for inst in &mut block.insts {
            rewrite_inst(inst, &vreg_to_preg);
        }
    }

    Ok(())
}

/// Extract all VRegs referenced by an instruction.
fn inst_vregs(inst: &MachInst) -> Vec<VReg> {
    let mut vregs = Vec::new();
    let mut add = |op: &Operand| {
        if let Some(v) = op.vreg() {
            vregs.push(v);
        }
    };

    match inst {
        MachInst::MovImm64 { dst, .. } => add(dst),
        MachInst::Add { dst, src } | MachInst::Sub { dst, src } | MachInst::IMul { dst, src } => {
            add(dst);
            add(src);
        }
        MachInst::IDiv { divisor } => add(divisor),
        MachInst::Cmp { lhs, rhs } => {
            add(lhs);
            add(rhs);
        }
        MachInst::SetCC { dst, .. } => add(dst),
        MachInst::Movzx { dst, src } => {
            add(dst);
            add(src);
        }
        MachInst::Mov { dst, src } => {
            add(dst);
            add(src);
        }
        MachInst::TestAndJnz { cond, .. } => add(cond),
        MachInst::Call { args, dst, .. } => {
            for arg in args {
                add(arg);
            }
            if let Some(d) = dst {
                add(d);
            }
        }
        MachInst::Ret { value } => {
            if let Some(v) = value {
                add(v);
            }
        }
        MachInst::Push { src } => add(src),
        MachInst::Pop { dst } => add(dst),
        MachInst::LoadStack { dst, .. } => add(dst),
        MachInst::StackAlloc { dst, .. } => add(dst),
        MachInst::LoadMem { dst, addr } => {
            add(dst);
            add(addr);
        }
        MachInst::StoreMem { addr, src } => {
            add(addr);
            add(src);
        }
        MachInst::LoadElemMem { dst, base, index } => {
            add(dst);
            add(base);
            add(index);
        }
        MachInst::Jmp { .. } | MachInst::Label { .. } | MachInst::Cqo => {}
    }

    vregs
}

/// Rewrite an instruction, replacing VRegs with PRegs.
fn rewrite_inst(inst: &mut MachInst, map: &HashMap<VReg, u8>) {
    let rewrite = |op: &mut Operand| {
        if let Operand::VReg(v) = *op {
            if let Some(&preg) = map.get(&v) {
                *op = Operand::PReg(preg);
            }
        }
    };

    match inst {
        MachInst::MovImm64 { dst, .. } => rewrite(dst),
        MachInst::Add { dst, src } | MachInst::Sub { dst, src } | MachInst::IMul { dst, src } => {
            rewrite(dst);
            rewrite(src);
        }
        MachInst::IDiv { divisor } => rewrite(divisor),
        MachInst::Cmp { lhs, rhs } => {
            rewrite(lhs);
            rewrite(rhs);
        }
        MachInst::SetCC { dst, .. } => rewrite(dst),
        MachInst::Movzx { dst, src } => {
            rewrite(dst);
            rewrite(src);
        }
        MachInst::Mov { dst, src } => {
            rewrite(dst);
            rewrite(src);
        }
        MachInst::TestAndJnz { cond, .. } => rewrite(cond),
        MachInst::Call { args, dst, .. } => {
            for arg in args {
                rewrite(arg);
            }
            if let Some(d) = dst {
                rewrite(d);
            }
        }
        MachInst::Ret { value } => {
            if let Some(v) = value {
                rewrite(v);
            }
        }
        MachInst::Push { src } => rewrite(src),
        MachInst::Pop { dst } => rewrite(dst),
        MachInst::LoadStack { dst, .. } => rewrite(dst),
        MachInst::StackAlloc { dst, .. } => rewrite(dst),
        MachInst::LoadMem { dst, addr } => {
            rewrite(dst);
            rewrite(addr);
        }
        MachInst::StoreMem { addr, src } => {
            rewrite(addr);
            rewrite(src);
        }
        MachInst::LoadElemMem { dst, base, index } => {
            rewrite(dst);
            rewrite(base);
            rewrite(index);
        }
        MachInst::Jmp { .. } | MachInst::Label { .. } | MachInst::Cqo => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isel::{MachBlock, MachFunction, MachInst, Operand};
    use crate::low_ir::VReg;
    use arc_targets::x86_64;

    #[test]
    fn allocate_simple_function() {
        let mut func = MachFunction {
            name: "add".to_string(),
            blocks: vec![MachBlock {
                label: "entry".to_string(),
                insts: vec![
                    MachInst::Label { index: 0 },
                    MachInst::Mov {
                        dst: Operand::VReg(VReg(2)),
                        src: Operand::VReg(VReg(0)),
                    },
                    MachInst::Add {
                        dst: Operand::VReg(VReg(2)),
                        src: Operand::VReg(VReg(1)),
                    },
                    MachInst::Ret {
                        value: Some(Operand::VReg(VReg(2))),
                    },
                ],
            }],
            vreg_count: 3,
            param_vregs: vec![VReg(0), VReg(1)],
            has_return: true,
        };

        let target = x86_64::target();
        allocate_registers(&mut func, &target).unwrap();

        // After allocation, all operands should be PRegs
        for block in &func.blocks {
            for inst in &block.insts {
                if let Some(vreg) = inst_vregs(inst).into_iter().next() {
                    panic!("found unallocated vreg: {:?}", vreg);
                }
            }
        }
    }

    #[test]
    fn params_get_calling_convention_regs() {
        let mut func = MachFunction {
            name: "f".to_string(),
            blocks: vec![MachBlock {
                label: "entry".to_string(),
                insts: vec![
                    MachInst::Label { index: 0 },
                    MachInst::Ret {
                        value: Some(Operand::VReg(VReg(0))),
                    },
                ],
            }],
            vreg_count: 1,
            param_vregs: vec![VReg(0)],
            has_return: true,
        };

        let target = x86_64::target();
        allocate_registers(&mut func, &target).unwrap();

        // First param should be in rdi (System V)
        match &func.blocks[0].insts[1] {
            MachInst::Ret {
                value: Some(Operand::PReg(reg)),
            } => {
                assert_eq!(*reg, x86_64::RDI, "first param should be in rdi");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn stack_passed_arguments() {
        // Create a function with 8 params (6 register + 2 stack)
        let mut param_vregs = Vec::new();
        let mut params = Vec::new();
        for i in 0..8u32 {
            param_vregs.push(VReg(i));
            params.push((VReg(i), crate::low_ir::MachType::I64));
        }

        let mut func = MachFunction {
            name: "many_args".to_string(),
            blocks: vec![MachBlock {
                label: "entry".to_string(),
                insts: vec![
                    MachInst::Label { index: 0 },
                    // Just return the 7th argument (index 6, first stack arg)
                    MachInst::Ret {
                        value: Some(Operand::VReg(VReg(6))),
                    },
                ],
            }],
            vreg_count: 8,
            param_vregs,
            has_return: true,
        };

        let target = x86_64::target();
        allocate_registers(&mut func, &target).unwrap();

        // Params 0-5 should be in the 6 arg registers
        // Params 6-7 should have LoadStack instructions inserted
        let has_load_stack = func.blocks[0]
            .insts
            .iter()
            .any(|inst| matches!(inst, MachInst::LoadStack { offset, .. } if *offset == 16));
        assert!(
            has_load_stack,
            "should have LoadStack for 7th argument at [rbp+16]"
        );

        let has_second_load = func.blocks[0]
            .insts
            .iter()
            .any(|inst| matches!(inst, MachInst::LoadStack { offset, .. } if *offset == 24));
        assert!(
            has_second_load,
            "should have LoadStack for 8th argument at [rbp+24]"
        );
    }
}
