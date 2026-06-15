//! x86_64 instruction encoding and object file emission.

use crate::isel::{CondCode, MachFunction, MachInst, Operand};
use crate::CodegenError;
use arc_object::ObjectFile;
use arc_targets::TargetDescription;
use std::collections::HashMap;

/// Encode a physical register for x86_64 ModR/M and REX fields.
fn reg_hw_enc(reg: u8, target: &TargetDescription) -> u8 {
    target.registers[reg as usize].hw_enc
}

fn needs_rex(reg: u8) -> bool {
    reg >= 8
}

/// Build a REX prefix byte.
/// W=1 for 64-bit operand size.
fn rex(w: bool, r_ext: bool, x_ext: bool, b_ext: bool) -> u8 {
    let mut byte = 0x40u8;
    if w {
        byte |= 0x08;
    }
    if r_ext {
        byte |= 0x04;
    }
    if x_ext {
        byte |= 0x02;
    }
    if b_ext {
        byte |= 0x01;
    }
    byte
}

/// ModR/M byte: mod=11 (register-register), reg, rm.
fn modrm_rr(reg: u8, rm: u8) -> u8 {
    0xC0 | ((reg & 7) << 3) | (rm & 7)
}

/// A pending call relocation: call instruction at code offset targeting callee.
struct CallFixup {
    /// Offset in code where the rel32 placeholder starts (after the 0xE8 byte).
    code_offset: usize,
    /// The callee function name.
    callee: String,
}

/// Emit machine code for all functions in the module.
pub fn emit_object(
    funcs: &[MachFunction],
    target: &TargetDescription,
) -> Result<ObjectFile, CodegenError> {
    let mut text = Vec::new();
    let mut symbols: Vec<(String, usize)> = Vec::new();
    let mut call_fixups: Vec<CallFixup> = Vec::new();

    for func in funcs {
        let offset = text.len();
        symbols.push((func.name.clone(), offset));
        emit_function(func, target, &mut text, &mut call_fixups)?;
    }

    // Resolve intra-module call fixups
    let sym_map: HashMap<&str, usize> = symbols.iter().map(|(n, o)| (n.as_str(), *o)).collect();
    for fixup in &call_fixups {
        if let Some(&target_offset) = sym_map.get(fixup.callee.as_str()) {
            let rel = (target_offset as i64) - (fixup.code_offset as i64 + 4);
            let rel32 = rel as i32;
            text[fixup.code_offset..fixup.code_offset + 4].copy_from_slice(&rel32.to_le_bytes());
        }
        // External calls remain as 0 — would need linker relocation entries
    }

    Ok(ObjectFile::elf64(text, symbols))
}

fn emit_function(
    func: &MachFunction,
    target: &TargetDescription,
    code: &mut Vec<u8>,
    call_fixups: &mut Vec<CallFixup>,
) -> Result<(), CodegenError> {
    let cc = &target.calling_convention;

    // Prologue: push rbp; mov rbp, rsp
    let rbp_enc = reg_hw_enc(cc.frame_pointer, target);
    let rsp_enc = reg_hw_enc(cc.stack_pointer, target);
    // push rbp
    code.push(0x50 + rbp_enc);
    // mov rbp, rsp — REX.W + 89 /r (mov r/m64, r64)
    code.push(rex(true, false, false, false));
    code.push(0x89);
    code.push(modrm_rr(rsp_enc, rbp_enc));

    // Record block offsets for jump fixups
    let mut block_offsets: HashMap<usize, usize> = HashMap::new();
    let mut fixups: Vec<(usize, usize, FixupKind)> = Vec::new(); // (code_offset, target_block, kind)

    for block in &func.blocks {
        for inst in &block.insts {
            match inst {
                MachInst::Label { index } => {
                    block_offsets.insert(*index, code.len());
                }
                MachInst::MovImm64 { dst, value } => {
                    let preg = extract_preg(dst)?;
                    let enc = reg_hw_enc(preg, target);
                    // movabs r64, imm64: REX.W + B8+rd io
                    code.push(rex(true, false, false, needs_rex(preg)));
                    code.push(0xB8 + (enc & 7));
                    code.extend_from_slice(&value.to_le_bytes());
                }
                MachInst::Add { dst, src } => {
                    let d = extract_preg(dst)?;
                    let s = extract_preg(src)?;
                    // add r64, r/m64: REX.W + 01 /r
                    code.push(rex(true, needs_rex(s), false, needs_rex(d)));
                    code.push(0x01);
                    code.push(modrm_rr(reg_hw_enc(s, target), reg_hw_enc(d, target)));
                }
                MachInst::Sub { dst, src } => {
                    let d = extract_preg(dst)?;
                    let s = extract_preg(src)?;
                    // sub r/m64, r64: REX.W + 29 /r
                    code.push(rex(true, needs_rex(s), false, needs_rex(d)));
                    code.push(0x29);
                    code.push(modrm_rr(reg_hw_enc(s, target), reg_hw_enc(d, target)));
                }
                MachInst::IMul { dst, src } => {
                    let d = extract_preg(dst)?;
                    let s = extract_preg(src)?;
                    // imul r64, r/m64: REX.W + 0F AF /r
                    code.push(rex(true, needs_rex(d), false, needs_rex(s)));
                    code.push(0x0F);
                    code.push(0xAF);
                    code.push(modrm_rr(reg_hw_enc(d, target), reg_hw_enc(s, target)));
                }
                MachInst::IDiv { divisor } => {
                    let s = extract_preg(divisor)?;
                    // idiv r/m64: REX.W + F7 /7
                    code.push(rex(true, false, false, needs_rex(s)));
                    code.push(0xF7);
                    code.push(modrm_rr(7, reg_hw_enc(s, target)));
                }
                MachInst::Cqo => {
                    // cqo: REX.W + 99
                    code.push(rex(true, false, false, false));
                    code.push(0x99);
                }
                MachInst::Cmp { lhs, rhs } => {
                    let l = extract_preg(lhs)?;
                    let r = extract_preg(rhs)?;
                    // cmp r/m64, r64: REX.W + 39 /r
                    code.push(rex(true, needs_rex(r), false, needs_rex(l)));
                    code.push(0x39);
                    code.push(modrm_rr(reg_hw_enc(r, target), reg_hw_enc(l, target)));
                }
                MachInst::SetCC { dst, cc: cond } => {
                    let d = extract_preg(dst)?;
                    let cc_byte = match cond {
                        CondCode::E => 0x94,
                        CondCode::Ne => 0x95,
                        CondCode::L => 0x9C,
                        CondCode::Le => 0x9E,
                        CondCode::G => 0x9F,
                        CondCode::Ge => 0x9D,
                    };
                    // setcc r/m8: 0F cc_byte /0
                    if needs_rex(d) {
                        code.push(rex(false, false, false, true));
                    }
                    code.push(0x0F);
                    code.push(cc_byte);
                    code.push(modrm_rr(0, reg_hw_enc(d, target)));
                }
                MachInst::Movzx { dst, src } => {
                    let d = extract_preg(dst)?;
                    let s = extract_preg(src)?;
                    // movzx r64, r/m8: REX.W + 0F B6 /r
                    code.push(rex(true, needs_rex(d), false, needs_rex(s)));
                    code.push(0x0F);
                    code.push(0xB6);
                    code.push(modrm_rr(reg_hw_enc(d, target), reg_hw_enc(s, target)));
                }
                MachInst::Mov { dst, src } => {
                    let d = extract_preg(dst)?;
                    let s = extract_preg(src)?;
                    if d == s {
                        continue; // Elide self-moves
                    }
                    // mov r/m64, r64: REX.W + 89 /r
                    code.push(rex(true, needs_rex(s), false, needs_rex(d)));
                    code.push(0x89);
                    code.push(modrm_rr(reg_hw_enc(s, target), reg_hw_enc(d, target)));
                }
                MachInst::Jmp { target: tgt } => {
                    // jmp rel32: E9 + rel32
                    code.push(0xE9);
                    fixups.push((code.len(), *tgt, FixupKind::Rel32));
                    code.extend_from_slice(&[0, 0, 0, 0]); // placeholder
                }
                MachInst::TestAndJnz {
                    cond,
                    true_target,
                    false_target,
                } => {
                    let c = extract_preg(cond)?;
                    let c_enc = reg_hw_enc(c, target);
                    // test r64, r64: REX.W + 85 /r
                    code.push(rex(true, needs_rex(c), false, needs_rex(c)));
                    code.push(0x85);
                    code.push(modrm_rr(c_enc, c_enc));
                    // jnz true_target: 0F 85 rel32
                    code.push(0x0F);
                    code.push(0x85);
                    fixups.push((code.len(), *true_target, FixupKind::Rel32));
                    code.extend_from_slice(&[0, 0, 0, 0]);
                    // jmp false_target: E9 rel32
                    code.push(0xE9);
                    fixups.push((code.len(), *false_target, FixupKind::Rel32));
                    code.extend_from_slice(&[0, 0, 0, 0]);
                }
                MachInst::Call { callee, args, dst } => {
                    // Move arguments into calling convention registers
                    for (i, arg) in args.iter().enumerate() {
                        if i < cc.int_arg_regs.len() {
                            let arg_preg = extract_preg(arg)?;
                            let cc_reg = cc.int_arg_regs[i];
                            if arg_preg != cc_reg {
                                let s_enc = reg_hw_enc(arg_preg, target);
                                let d_enc = reg_hw_enc(cc_reg, target);
                                code.push(rex(true, needs_rex(arg_preg), false, needs_rex(cc_reg)));
                                code.push(0x89);
                                code.push(modrm_rr(s_enc, d_enc));
                            }
                        }
                    }
                    // call rel32 — placeholder to be fixed up
                    code.push(0xE8);
                    let fixup_offset = code.len();
                    code.extend_from_slice(&[0, 0, 0, 0]); // rel32 placeholder
                    call_fixups.push(CallFixup {
                        code_offset: fixup_offset,
                        callee: callee.clone(),
                    });

                    // Move return value if needed
                    if let Some(dst_op) = dst {
                        let d = extract_preg(dst_op)?;
                        let ret_reg = cc.int_ret_reg;
                        if d != ret_reg {
                            let s_enc = reg_hw_enc(ret_reg, target);
                            let d_enc = reg_hw_enc(d, target);
                            code.push(rex(true, needs_rex(ret_reg), false, needs_rex(d)));
                            code.push(0x89);
                            code.push(modrm_rr(s_enc, d_enc));
                        }
                    }
                }
                MachInst::Ret { value } => {
                    // Move return value to rax if not already there
                    if let Some(val) = value {
                        let v = extract_preg(val)?;
                        let ret_reg = cc.int_ret_reg;
                        if v != ret_reg {
                            let s_enc = reg_hw_enc(v, target);
                            let d_enc = reg_hw_enc(ret_reg, target);
                            code.push(rex(true, needs_rex(v), false, needs_rex(ret_reg)));
                            code.push(0x89);
                            code.push(modrm_rr(s_enc, d_enc));
                        }
                    }
                    // Epilogue: pop rbp; ret
                    code.push(0x5D); // pop rbp
                    code.push(0xC3); // ret
                }
                MachInst::Push { src } => {
                    let s = extract_preg(src)?;
                    let enc = reg_hw_enc(s, target);
                    if needs_rex(s) {
                        code.push(rex(false, false, false, true));
                    }
                    code.push(0x50 + (enc & 7));
                }
                MachInst::Pop { dst } => {
                    let d = extract_preg(dst)?;
                    let enc = reg_hw_enc(d, target);
                    if needs_rex(d) {
                        code.push(rex(false, false, false, true));
                    }
                    code.push(0x58 + (enc & 7));
                }
                MachInst::StackAlloc { dst, size } => {
                    // sub rsp, size
                    let rsp = cc.stack_pointer;
                    let rsp_enc = reg_hw_enc(rsp, target);
                    // REX.W + 81 /5 imm32 (sub r/m64, imm32)
                    code.push(rex(true, false, false, needs_rex(rsp)));
                    code.push(0x81);
                    code.push(modrm_rr(5, rsp_enc));
                    code.extend_from_slice(&size.to_le_bytes());

                    // lea dst, [rsp] — get pointer to allocated space
                    let d = extract_preg(dst)?;
                    let d_enc = reg_hw_enc(d, target);
                    // REX.W + 8D /r with ModRM for [rsp] (needs SIB byte)
                    code.push(rex(true, needs_rex(d), false, needs_rex(rsp)));
                    code.push(0x8D);
                    // ModRM: mod=00, reg=dst, rm=100 (SIB follows)
                    code.push(((d_enc & 7) << 3) | 0x04);
                    // SIB: scale=00, index=100 (none), base=rsp
                    code.push(0x24);
                }
                MachInst::LoadMem { dst, addr } => {
                    // mov dst, [addr]
                    let d = extract_preg(dst)?;
                    let a = extract_preg(addr)?;
                    let d_enc = reg_hw_enc(d, target);
                    let a_enc = reg_hw_enc(a, target);
                    // REX.W + 8B /r with ModRM mod=00 (indirect)
                    code.push(rex(true, needs_rex(d), false, needs_rex(a)));
                    code.push(0x8B);
                    if a_enc & 7 == 4 {
                        // rsp-based addressing needs SIB
                        code.push(((d_enc & 7) << 3) | 0x04);
                        code.push(0x24);
                    } else if a_enc & 7 == 5 {
                        // rbp-based addressing with mod=00 is rip-relative,
                        // use mod=01 with disp8=0
                        code.push(0x40 | ((d_enc & 7) << 3) | (a_enc & 7));
                        code.push(0x00);
                    } else {
                        code.push(((d_enc & 7) << 3) | (a_enc & 7));
                    }
                }
                MachInst::StoreMem { addr, src } => {
                    // mov [addr], src
                    let a = extract_preg(addr)?;
                    let s = extract_preg(src)?;
                    let a_enc = reg_hw_enc(a, target);
                    let s_enc = reg_hw_enc(s, target);
                    // REX.W + 89 /r with ModRM mod=00 (indirect)
                    code.push(rex(true, needs_rex(s), false, needs_rex(a)));
                    code.push(0x89);
                    if a_enc & 7 == 4 {
                        code.push(((s_enc & 7) << 3) | 0x04);
                        code.push(0x24);
                    } else if a_enc & 7 == 5 {
                        code.push(0x40 | ((s_enc & 7) << 3) | (a_enc & 7));
                        code.push(0x00);
                    } else {
                        code.push(((s_enc & 7) << 3) | (a_enc & 7));
                    }
                }
                MachInst::LoadElemMem { dst, base, index } => {
                    // mov dst, [base + index*8]
                    let d = extract_preg(dst)?;
                    let b = extract_preg(base)?;
                    let i = extract_preg(index)?;
                    let d_enc = reg_hw_enc(d, target);
                    let b_enc = reg_hw_enc(b, target);
                    let i_enc = reg_hw_enc(i, target);
                    // REX.W + 8B /r with SIB (scale=8)
                    code.push(rex(true, needs_rex(d), needs_rex(i), needs_rex(b)));
                    code.push(0x8B);
                    // ModRM: mod=00, reg=dst, rm=100 (SIB)
                    code.push(((d_enc & 7) << 3) | 0x04);
                    // SIB: scale=11 (×8), index, base
                    code.push(0xC0 | ((i_enc & 7) << 3) | (b_enc & 7));
                }
                MachInst::LoadStack { dst, offset } => {
                    // mov dst, [rbp + offset]
                    // REX.W + 8B /r with ModR/M for [rbp+disp32]
                    let d = extract_preg(dst)?;
                    let d_enc = reg_hw_enc(d, target);
                    let rbp_enc = reg_hw_enc(target.calling_convention.frame_pointer, target);
                    code.push(rex(
                        true,
                        needs_rex(d),
                        false,
                        needs_rex(target.calling_convention.frame_pointer),
                    ));
                    code.push(0x8B); // mov r64, r/m64
                                     // ModR/M: mod=10 (disp32), reg=dst, rm=rbp
                    code.push(0x80 | ((d_enc & 7) << 3) | (rbp_enc & 7));
                    // rbp as base requires SIB when rm=5 in certain modes,
                    // but mod=10 with rm=101 (rbp) uses [rbp+disp32] directly
                    code.extend_from_slice(&offset.to_le_bytes());
                }
            }
        }
    }

    // Apply jump fixups
    for (fixup_offset, target_block, kind) in &fixups {
        if let Some(&target_offset) = block_offsets.get(target_block) {
            match kind {
                FixupKind::Rel32 => {
                    let rel = (target_offset as i64) - (*fixup_offset as i64 + 4);
                    let rel32 = rel as i32;
                    code[*fixup_offset..*fixup_offset + 4].copy_from_slice(&rel32.to_le_bytes());
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug)]
enum FixupKind {
    Rel32,
}

fn extract_preg(op: &Operand) -> Result<u8, CodegenError> {
    match op {
        Operand::PReg(r) => Ok(*r),
        Operand::VReg(v) => Err(CodegenError::Failed(format!(
            "unallocated virtual register {} in emit phase",
            v
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isel::{MachBlock, MachFunction, MachInst, Operand};
    use arc_targets::x86_64;

    #[test]
    fn emit_simple_return() {
        let func = MachFunction {
            name: "ret42".to_string(),
            blocks: vec![MachBlock {
                label: "entry".to_string(),
                insts: vec![
                    MachInst::Label { index: 0 },
                    MachInst::MovImm64 {
                        dst: Operand::PReg(x86_64::RAX),
                        value: 42,
                    },
                    MachInst::Ret {
                        value: Some(Operand::PReg(x86_64::RAX)),
                    },
                ],
            }],
            vreg_count: 0,
            param_vregs: vec![],
            has_return: true,
        };

        let target = x86_64::target();
        let obj = emit_object(&[func], &target).unwrap();
        let code = &obj.text;

        // Should start with push rbp (0x55); mov rbp, rsp (REX.W 89 E5)
        assert_eq!(code[0], 0x55, "push rbp");
        assert_eq!(code[1], 0x48, "REX.W");
        assert_eq!(code[2], 0x89, "mov");

        // Should end with pop rbp (0x5D); ret (0xC3)
        let len = code.len();
        assert_eq!(code[len - 1], 0xC3, "ret");
        assert_eq!(code[len - 2], 0x5D, "pop rbp");
    }

    #[test]
    fn emit_add_function() {
        // Emit a function that adds rdi + rsi and returns in rax
        let func = MachFunction {
            name: "add".to_string(),
            blocks: vec![MachBlock {
                label: "entry".to_string(),
                insts: vec![
                    MachInst::Label { index: 0 },
                    MachInst::Mov {
                        dst: Operand::PReg(x86_64::RAX),
                        src: Operand::PReg(x86_64::RDI),
                    },
                    MachInst::Add {
                        dst: Operand::PReg(x86_64::RAX),
                        src: Operand::PReg(x86_64::RSI),
                    },
                    MachInst::Ret {
                        value: Some(Operand::PReg(x86_64::RAX)),
                    },
                ],
            }],
            vreg_count: 0,
            param_vregs: vec![],
            has_return: true,
        };

        let target = x86_64::target();
        let obj = emit_object(&[func], &target).unwrap();

        // Verify it's non-empty and has prologue/epilogue
        assert!(obj.text.len() > 6);
        assert_eq!(obj.text[0], 0x55); // push rbp
        assert_eq!(*obj.text.last().unwrap(), 0xC3); // ret
        assert_eq!(obj.symbols.len(), 1);
        assert_eq!(obj.symbols[0].0, "add");
    }

    #[test]
    fn emit_produces_valid_elf() {
        let func = MachFunction {
            name: "nop".to_string(),
            blocks: vec![MachBlock {
                label: "entry".to_string(),
                insts: vec![MachInst::Label { index: 0 }, MachInst::Ret { value: None }],
            }],
            vreg_count: 0,
            param_vregs: vec![],
            has_return: false,
        };

        let target = x86_64::target();
        let obj = emit_object(&[func], &target).unwrap();
        let elf = obj.to_elf();

        // ELF magic bytes
        assert_eq!(&elf[0..4], b"\x7fELF");
        // ELF class: 64-bit
        assert_eq!(elf[4], 2);
        // ELF data: little-endian
        assert_eq!(elf[5], 1);
    }

    #[test]
    fn call_fixups_resolved() {
        // Two functions: "callee" returns 42, "caller" calls "callee"
        let callee = MachFunction {
            name: "callee".to_string(),
            blocks: vec![MachBlock {
                label: "entry".to_string(),
                insts: vec![
                    MachInst::Label { index: 0 },
                    MachInst::MovImm64 {
                        dst: Operand::PReg(x86_64::RAX),
                        value: 42,
                    },
                    MachInst::Ret {
                        value: Some(Operand::PReg(x86_64::RAX)),
                    },
                ],
            }],
            vreg_count: 0,
            param_vregs: vec![],
            has_return: true,
        };

        let caller = MachFunction {
            name: "caller".to_string(),
            blocks: vec![MachBlock {
                label: "entry".to_string(),
                insts: vec![
                    MachInst::Label { index: 0 },
                    MachInst::Call {
                        callee: "callee".to_string(),
                        args: vec![],
                        dst: Some(Operand::PReg(x86_64::RAX)),
                    },
                    MachInst::Ret {
                        value: Some(Operand::PReg(x86_64::RAX)),
                    },
                ],
            }],
            vreg_count: 0,
            param_vregs: vec![],
            has_return: true,
        };

        let target = x86_64::target();
        let obj = emit_object(&[callee, caller], &target).unwrap();

        // Find the call instruction in the text: 0xE8 followed by rel32
        let caller_start = obj.symbols[1].1;
        let code = &obj.text;
        // Look for 0xE8 after caller's prologue
        let call_pos = (caller_start..code.len())
            .find(|&i| code[i] == 0xE8)
            .expect("should find call instruction");

        // The rel32 should NOT be all zeros (it was fixed up)
        let rel32_bytes = &code[call_pos + 1..call_pos + 5];
        let rel32 = i32::from_le_bytes([
            rel32_bytes[0],
            rel32_bytes[1],
            rel32_bytes[2],
            rel32_bytes[3],
        ]);

        // rel32 should point backwards to callee (negative offset)
        assert!(
            rel32 < 0,
            "call to callee should have negative relative offset, got {}",
            rel32
        );

        // Verify the target: call_pos + 5 + rel32 should equal callee start (0)
        let target_addr = (call_pos as i64 + 5 + rel32 as i64) as usize;
        assert_eq!(
            target_addr, obj.symbols[0].1,
            "call should resolve to callee"
        );
    }

    #[test]
    fn emit_memory_ops() {
        // Test that memory instructions emit valid machine code
        let func = MachFunction {
            name: "mem_test".to_string(),
            blocks: vec![MachBlock {
                label: "entry".to_string(),
                insts: vec![
                    MachInst::Label { index: 0 },
                    MachInst::StackAlloc {
                        dst: Operand::PReg(x86_64::RAX),
                        size: 8,
                    },
                    MachInst::MovImm64 {
                        dst: Operand::PReg(x86_64::RCX),
                        value: 99,
                    },
                    MachInst::StoreMem {
                        addr: Operand::PReg(x86_64::RAX),
                        src: Operand::PReg(x86_64::RCX),
                    },
                    MachInst::LoadMem {
                        dst: Operand::PReg(x86_64::RDX),
                        addr: Operand::PReg(x86_64::RAX),
                    },
                    MachInst::Ret {
                        value: Some(Operand::PReg(x86_64::RDX)),
                    },
                ],
            }],
            vreg_count: 0,
            param_vregs: vec![],
            has_return: true,
        };

        let target = x86_64::target();
        let obj = emit_object(&[func], &target).unwrap();

        // Should compile without error and produce non-trivial code
        assert!(
            obj.text.len() > 20,
            "memory ops should produce substantial code"
        );
        assert_eq!(obj.text[0], 0x55, "push rbp");
        assert_eq!(*obj.text.last().unwrap(), 0xC3, "ret");
    }
}
