//! WebAssembly binary emitter.
//!
//! Produces a valid `.wasm` module from low IR functions. Since wasm is a
//! stack machine we translate directly from `LowFunction` rather than going
//! through the register-machine isel/regalloc pipeline.

use crate::low_ir::{CmpOp, LowFunction, LowOp, MachType};
use crate::CodegenError;
use arc_targets::wasm32::*;
use std::collections::HashMap;

// Wasm binary constants
const WASM_MAGIC: &[u8] = b"\0asm";
const WASM_VERSION: &[u8] = &[1, 0, 0, 0];

// Section IDs
const SECTION_TYPE: u8 = 1;
const SECTION_FUNCTION: u8 = 3;
const SECTION_EXPORT: u8 = 7;
const SECTION_CODE: u8 = 10;

// Value types
const VALTYPE_I32: u8 = 0x7F;
const VALTYPE_I64: u8 = 0x7E;

// Block type for void blocks
const BLOCKTYPE_VOID: u8 = 0x40;

fn valtype_for(mty: MachType) -> u8 {
    match mty {
        MachType::I64 => VALTYPE_I64,
        MachType::I32 | MachType::I1 => VALTYPE_I32,
    }
}

/// Emit a complete wasm binary module from low IR functions.
pub fn emit_wasm_module(funcs: &[LowFunction]) -> Result<Vec<u8>, CodegenError> {
    let mut wasm = Vec::new();

    // Header
    wasm.extend_from_slice(WASM_MAGIC);
    wasm.extend_from_slice(WASM_VERSION);

    // Type section: one type per function
    let type_section = build_type_section(funcs);
    emit_section(&mut wasm, SECTION_TYPE, &type_section);

    // Function section: maps function index → type index (1:1)
    let func_section = build_function_section(funcs);
    emit_section(&mut wasm, SECTION_FUNCTION, &func_section);

    // Export section: export all functions
    let export_section = build_export_section(funcs);
    emit_section(&mut wasm, SECTION_EXPORT, &export_section);

    // Build function name → index mapping for call resolution
    let func_index: HashMap<String, u32> = funcs
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.clone(), i as u32))
        .collect();

    // Code section: function bodies
    let code_section = build_code_section(funcs, &func_index)?;
    emit_section(&mut wasm, SECTION_CODE, &code_section);

    Ok(wasm)
}

fn emit_section(out: &mut Vec<u8>, id: u8, data: &[u8]) {
    out.push(id);
    out.extend_from_slice(&encode_uleb128(data.len() as u64));
    out.extend_from_slice(data);
}

/// Build the type section containing function signatures.
fn build_type_section(funcs: &[LowFunction]) -> Vec<u8> {
    let mut section = Vec::new();
    section.extend_from_slice(&encode_uleb128(funcs.len() as u64));

    for func in funcs {
        // functype tag
        section.push(0x60);
        // param types
        section.extend_from_slice(&encode_uleb128(func.params.len() as u64));
        for (_vreg, mty) in &func.params {
            section.push(valtype_for(*mty));
        }
        // result types
        match func.result_type {
            Some(mty) => {
                section.extend_from_slice(&encode_uleb128(1u64));
                section.push(valtype_for(mty));
            }
            None => {
                section.extend_from_slice(&encode_uleb128(0u64));
            }
        }
    }
    section
}

/// Build the function section (type index per function).
fn build_function_section(funcs: &[LowFunction]) -> Vec<u8> {
    let mut section = Vec::new();
    section.extend_from_slice(&encode_uleb128(funcs.len() as u64));
    for (i, _) in funcs.iter().enumerate() {
        section.extend_from_slice(&encode_uleb128(i as u64));
    }
    section
}

/// Build the export section.
fn build_export_section(funcs: &[LowFunction]) -> Vec<u8> {
    let mut section = Vec::new();
    section.extend_from_slice(&encode_uleb128(funcs.len() as u64));
    for (i, func) in funcs.iter().enumerate() {
        // name
        let name_bytes = func.name.as_bytes();
        section.extend_from_slice(&encode_uleb128(name_bytes.len() as u64));
        section.extend_from_slice(name_bytes);
        // export kind: function = 0x00
        section.push(0x00);
        // function index
        section.extend_from_slice(&encode_uleb128(i as u64));
    }
    section
}

/// Build the code section with all function bodies.
fn build_code_section(
    funcs: &[LowFunction],
    func_index: &HashMap<String, u32>,
) -> Result<Vec<u8>, CodegenError> {
    let mut section = Vec::new();
    section.extend_from_slice(&encode_uleb128(funcs.len() as u64));

    for func in funcs {
        let body = emit_function_body(func, func_index)?;
        section.extend_from_slice(&encode_uleb128(body.len() as u64));
        section.extend_from_slice(&body);
    }
    Ok(section)
}

/// Emit a single function body (locals declaration + code + end).
fn emit_function_body(
    func: &LowFunction,
    func_index: &HashMap<String, u32>,
) -> Result<Vec<u8>, CodegenError> {
    let mut body = Vec::new();

    // Count locals needed: all vregs beyond the params
    let num_params = func.params.len() as u32;
    let total_vregs = func.vreg_count();
    let num_locals = total_vregs.saturating_sub(num_params);

    // Locals declaration: one group of i64 locals
    if num_locals > 0 {
        body.extend_from_slice(&encode_uleb128(1u64)); // one local group
        body.extend_from_slice(&encode_uleb128(num_locals as u64));
        body.push(VALTYPE_I64); // all locals are i64
    } else {
        body.extend_from_slice(&encode_uleb128(0u64)); // no local groups
    }

    // For single-block functions, emit directly.
    // For multi-block functions, use a block-per-basic-block translation.
    if func.blocks.len() == 1 {
        emit_block_ops(&func.blocks[0].ops, &mut body, func, func_index)?;
    } else {
        // Multi-block: wrap in nested wasm blocks for forward jumps.
        // Each AIR block gets a wasm `block` label.
        // Block 0 is the entry and executes first.
        // For simplicity, we emit all blocks sequentially wrapped in
        // wasm blocks so `br N` can jump forward.

        // Open N-1 wasm blocks (one for each non-entry block as a jump target)
        let n = func.blocks.len();
        for _ in 1..n {
            body.push(OP_BLOCK);
            body.push(BLOCKTYPE_VOID);
        }

        // Emit blocks in order
        for (block_idx, block) in func.blocks.iter().enumerate() {
            emit_block_ops(&block.ops, &mut body, func, func_index)?;
            // Close the wasm block for this target (except the last one)
            if block_idx < n - 1 {
                body.push(OP_END);
            }
        }
    }

    // Function end
    body.push(OP_END);
    Ok(body)
}

/// Emit wasm instructions for a sequence of low-IR ops.
fn emit_block_ops(
    ops: &[LowOp],
    out: &mut Vec<u8>,
    func: &LowFunction,
    func_index: &HashMap<String, u32>,
) -> Result<(), CodegenError> {
    for op in ops {
        match op {
            LowOp::LoadImm { dst, value } => {
                out.push(OP_I64_CONST);
                out.extend_from_slice(&encode_sleb128(*value));
                out.push(OP_LOCAL_SET);
                out.extend_from_slice(&encode_uleb128(dst.index() as u64));
            }
            LowOp::Add { dst, lhs, rhs } => {
                emit_local_get(out, lhs.index());
                emit_local_get(out, rhs.index());
                out.push(OP_I64_ADD);
                emit_local_set(out, dst.index());
            }
            LowOp::Sub { dst, lhs, rhs } => {
                emit_local_get(out, lhs.index());
                emit_local_get(out, rhs.index());
                out.push(OP_I64_SUB);
                emit_local_set(out, dst.index());
            }
            LowOp::Mul { dst, lhs, rhs } => {
                emit_local_get(out, lhs.index());
                emit_local_get(out, rhs.index());
                out.push(OP_I64_MUL);
                emit_local_set(out, dst.index());
            }
            LowOp::Div { dst, lhs, rhs } => {
                emit_local_get(out, lhs.index());
                emit_local_get(out, rhs.index());
                out.push(OP_I64_DIV_S);
                emit_local_set(out, dst.index());
            }
            LowOp::Cmp { dst, op, lhs, rhs } => {
                emit_local_get(out, lhs.index());
                emit_local_get(out, rhs.index());
                let cmp_op = match op {
                    CmpOp::Eq => OP_I64_EQ,
                    CmpOp::Ne => OP_I64_NE,
                    CmpOp::Slt => OP_I64_LT_S,
                    CmpOp::Sle => OP_I64_LE_S,
                    CmpOp::Sgt => OP_I64_GT_S,
                    CmpOp::Sge => OP_I64_GE_S,
                };
                out.push(cmp_op);
                // i64 compare produces i32 on the wasm stack; extend to i64
                out.push(0xAD); // i64.extend_i32_u
                emit_local_set(out, dst.index());
            }
            LowOp::Copy { dst, src } => {
                emit_local_get(out, src.index());
                emit_local_set(out, dst.index());
            }
            LowOp::Jump { target } => {
                // In the nested-block scheme, jumping forward to block `target`
                // means `br (n_blocks - 1 - target)` to reach the right label.
                let depth = func.blocks.len() - 1 - target;
                out.push(OP_BR);
                out.extend_from_slice(&encode_uleb128(depth as u64));
            }
            LowOp::CondJump {
                cond,
                true_target,
                false_target,
            } => {
                // Branch-if to true, fall through to false
                emit_local_get(out, cond.index());
                // Truncate i64 cond to i32 for br_if
                out.push(0xA7); // i32.wrap_i64
                let true_depth = func.blocks.len() - 1 - true_target;
                out.push(OP_BR_IF);
                out.extend_from_slice(&encode_uleb128(true_depth as u64));
                // Unconditional branch to false target
                let false_depth = func.blocks.len() - 1 - false_target;
                out.push(OP_BR);
                out.extend_from_slice(&encode_uleb128(false_depth as u64));
            }
            LowOp::Call { dst, callee, args } => {
                // Push args onto the stack
                for arg in args {
                    emit_local_get(out, arg.index());
                }
                // Resolve callee name to function index
                let idx = func_index.get(callee).ok_or_else(|| {
                    CodegenError::Unsupported(format!(
                        "call to unknown function '{}' (not in module)",
                        callee
                    ))
                })?;
                out.push(OP_CALL);
                out.extend_from_slice(&encode_uleb128(*idx as u64));
                // Store result if any
                if let Some(d) = dst {
                    emit_local_set(out, d.index());
                }
            }
            LowOp::Ret { value } => {
                if let Some(v) = value {
                    emit_local_get(out, v.index());
                }
                out.push(OP_RETURN);
            }
            LowOp::StackAlloc { dst, size } => {
                // Wasm linear memory: allocate by bumping a global pointer.
                // For simplicity, use a local as a fake stack pointer.
                // Store current "stack pointer" (0 initially) in dst.
                out.push(OP_I64_CONST);
                out.extend_from_slice(&encode_sleb128(*size));
                emit_local_set(out, dst.index());
            }
            LowOp::MemLoad { dst, addr } => {
                // i64.load from wasm linear memory
                emit_local_get(out, addr.index());
                // i32.wrap_i64 (wasm memory ops need i32 addresses)
                out.push(0xA7);
                // i64.load align=3 offset=0
                out.push(0x29); // i64.load
                out.push(0x03); // align (2^3 = 8)
                out.push(0x00); // offset
                emit_local_set(out, dst.index());
            }
            LowOp::MemStore { addr, src } => {
                emit_local_get(out, addr.index());
                out.push(0xA7); // i32.wrap_i64
                emit_local_get(out, src.index());
                // i64.store align=3 offset=0
                out.push(0x37); // i64.store
                out.push(0x03); // align
                out.push(0x00); // offset
            }
            LowOp::MemLoadElem { dst, base, index } => {
                // base + index * 8
                emit_local_get(out, base.index());
                emit_local_get(out, index.index());
                out.push(OP_I64_CONST);
                out.extend_from_slice(&encode_sleb128(8));
                out.push(OP_I64_MUL);
                out.push(OP_I64_ADD);
                out.push(0xA7); // i32.wrap_i64
                out.push(0x29); // i64.load
                out.push(0x03); // align
                out.push(0x00); // offset
                emit_local_set(out, dst.index());
            }
        }
    }
    Ok(())
}

fn emit_local_get(out: &mut Vec<u8>, index: u32) {
    out.push(OP_LOCAL_GET);
    out.extend_from_slice(&encode_uleb128(index as u64));
}

fn emit_local_set(out: &mut Vec<u8>, index: u32) {
    out.push(OP_LOCAL_SET);
    out.extend_from_slice(&encode_uleb128(index as u64));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::low_ir::{LowBlock, LowFunction, LowOp, MachType, VReg};

    fn make_simple_func(name: &str, ops: Vec<LowOp>) -> LowFunction {
        LowFunction {
            name: name.to_string(),
            params: vec![],
            result_type: Some(MachType::I64),
            blocks: vec![LowBlock {
                label: "entry".to_string(),
                params: vec![],
                ops,
            }],
            next_vreg: 4,
        }
    }

    #[test]
    fn emit_wasm_magic_and_version() {
        let func = make_simple_func(
            "main",
            vec![
                LowOp::LoadImm {
                    dst: VReg(0),
                    value: 42,
                },
                LowOp::Ret {
                    value: Some(VReg(0)),
                },
            ],
        );
        let wasm = emit_wasm_module(&[func]).unwrap();
        assert_eq!(&wasm[0..4], b"\0asm");
        assert_eq!(&wasm[4..8], &[1, 0, 0, 0]);
    }

    #[test]
    fn emit_wasm_has_all_sections() {
        let func = make_simple_func(
            "main",
            vec![
                LowOp::LoadImm {
                    dst: VReg(0),
                    value: 42,
                },
                LowOp::Ret {
                    value: Some(VReg(0)),
                },
            ],
        );
        let wasm = emit_wasm_module(&[func]).unwrap();

        // Check that all 4 section IDs appear
        let mut pos = 8;
        let mut section_ids = Vec::new();
        while pos < wasm.len() {
            let id = wasm[pos];
            section_ids.push(id);
            pos += 1;
            // Read section size (LEB128)
            let mut size = 0u64;
            let mut shift = 0;
            loop {
                let byte = wasm[pos];
                pos += 1;
                size |= ((byte & 0x7f) as u64) << shift;
                shift += 7;
                if byte & 0x80 == 0 {
                    break;
                }
            }
            pos += size as usize;
        }
        assert!(section_ids.contains(&SECTION_TYPE), "missing type section");
        assert!(
            section_ids.contains(&SECTION_FUNCTION),
            "missing function section"
        );
        assert!(
            section_ids.contains(&SECTION_EXPORT),
            "missing export section"
        );
        assert!(section_ids.contains(&SECTION_CODE), "missing code section");
    }

    #[test]
    fn emit_wasm_simple_return() {
        let func = make_simple_func(
            "answer",
            vec![
                LowOp::LoadImm {
                    dst: VReg(0),
                    value: 42,
                },
                LowOp::Ret {
                    value: Some(VReg(0)),
                },
            ],
        );
        let wasm = emit_wasm_module(&[func]).unwrap();
        assert!(wasm.len() > 8);

        // Verify the export name "answer" appears in the binary
        assert!(wasm.windows(6).any(|w| w == b"answer"));
    }

    #[test]
    fn emit_wasm_add_two_params() {
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
        let wasm = emit_wasm_module(&[func]).unwrap();
        assert!(wasm.len() > 8);

        // The wasm should contain local.get 0, local.get 1, i64.add in sequence
        let code_bytes: Vec<u8> = vec![OP_LOCAL_GET, 0, OP_LOCAL_GET, 1, OP_I64_ADD];
        assert!(
            wasm.windows(code_bytes.len())
                .any(|w| w == code_bytes.as_slice()),
            "wasm binary should contain local.get 0, local.get 1, i64.add sequence"
        );
    }

    #[test]
    fn emit_wasm_multiple_functions() {
        let f1 = make_simple_func(
            "foo",
            vec![
                LowOp::LoadImm {
                    dst: VReg(0),
                    value: 1,
                },
                LowOp::Ret {
                    value: Some(VReg(0)),
                },
            ],
        );
        let f2 = make_simple_func(
            "bar",
            vec![
                LowOp::LoadImm {
                    dst: VReg(0),
                    value: 2,
                },
                LowOp::Ret {
                    value: Some(VReg(0)),
                },
            ],
        );
        let wasm = emit_wasm_module(&[f1, f2]).unwrap();

        // Both function names should appear in the export section
        assert!(wasm.windows(3).any(|w| w == b"foo"));
        assert!(wasm.windows(3).any(|w| w == b"bar"));
    }

    #[test]
    fn emit_wasm_call_resolves_function_index() {
        // "helper" is function index 0, "main" is function index 1
        let helper = make_simple_func(
            "helper",
            vec![
                LowOp::LoadImm {
                    dst: VReg(0),
                    value: 42,
                },
                LowOp::Ret {
                    value: Some(VReg(0)),
                },
            ],
        );
        let main = LowFunction {
            name: "main".to_string(),
            params: vec![],
            result_type: Some(MachType::I64),
            blocks: vec![LowBlock {
                label: "entry".to_string(),
                params: vec![],
                ops: vec![
                    LowOp::Call {
                        dst: Some(VReg(0)),
                        callee: "helper".to_string(),
                        args: vec![],
                    },
                    LowOp::Ret {
                        value: Some(VReg(0)),
                    },
                ],
            }],
            next_vreg: 4,
        };
        let wasm = emit_wasm_module(&[helper, main]).unwrap();

        // The call instruction should reference function index 0 (helper),
        // not a hardcoded placeholder. Find OP_CALL followed by index 0.
        let call_seq: Vec<u8> = vec![OP_CALL, 0x00];
        assert!(
            wasm.windows(call_seq.len())
                .any(|w| w == call_seq.as_slice()),
            "wasm should contain call to function index 0 (helper)"
        );
    }

    #[test]
    fn emit_wasm_call_second_function_index() {
        // "first" is index 0, "second" is index 1, "caller" is index 2
        let first = make_simple_func(
            "first",
            vec![
                LowOp::LoadImm {
                    dst: VReg(0),
                    value: 1,
                },
                LowOp::Ret {
                    value: Some(VReg(0)),
                },
            ],
        );
        let second = make_simple_func(
            "second",
            vec![
                LowOp::LoadImm {
                    dst: VReg(0),
                    value: 2,
                },
                LowOp::Ret {
                    value: Some(VReg(0)),
                },
            ],
        );
        let caller = LowFunction {
            name: "caller".to_string(),
            params: vec![],
            result_type: Some(MachType::I64),
            blocks: vec![LowBlock {
                label: "entry".to_string(),
                params: vec![],
                ops: vec![
                    LowOp::Call {
                        dst: Some(VReg(0)),
                        callee: "second".to_string(),
                        args: vec![],
                    },
                    LowOp::Ret {
                        value: Some(VReg(0)),
                    },
                ],
            }],
            next_vreg: 4,
        };
        let wasm = emit_wasm_module(&[first, second, caller]).unwrap();

        // The call should reference function index 1 (second), not 0
        let call_seq: Vec<u8> = vec![OP_CALL, 0x01];
        assert!(
            wasm.windows(call_seq.len())
                .any(|w| w == call_seq.as_slice()),
            "wasm should contain call to function index 1 (second)"
        );
    }

    #[test]
    fn emit_wasm_call_unknown_function_errors() {
        let func = LowFunction {
            name: "main".to_string(),
            params: vec![],
            result_type: Some(MachType::I64),
            blocks: vec![LowBlock {
                label: "entry".to_string(),
                params: vec![],
                ops: vec![
                    LowOp::Call {
                        dst: Some(VReg(0)),
                        callee: "nonexistent".to_string(),
                        args: vec![],
                    },
                    LowOp::Ret {
                        value: Some(VReg(0)),
                    },
                ],
            }],
            next_vreg: 4,
        };
        let err = emit_wasm_module(&[func]).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("nonexistent"),
            "error should name the missing function: {}",
            msg
        );
    }

    #[test]
    fn emit_wasm_void_function() {
        let func = LowFunction {
            name: "noop".to_string(),
            params: vec![],
            result_type: None,
            blocks: vec![LowBlock {
                label: "entry".to_string(),
                params: vec![],
                ops: vec![LowOp::Ret { value: None }],
            }],
            next_vreg: 0,
        };
        let wasm = emit_wasm_module(&[func]).unwrap();
        assert!(wasm.len() > 8);
    }

    #[test]
    fn emit_wasm_comparison() {
        let func = make_simple_func(
            "cmp",
            vec![
                LowOp::LoadImm {
                    dst: VReg(0),
                    value: 10,
                },
                LowOp::LoadImm {
                    dst: VReg(1),
                    value: 20,
                },
                LowOp::Cmp {
                    dst: VReg(2),
                    op: CmpOp::Eq,
                    lhs: VReg(0),
                    rhs: VReg(1),
                },
                LowOp::Ret {
                    value: Some(VReg(2)),
                },
            ],
        );
        let wasm = emit_wasm_module(&[func]).unwrap();
        // Should contain i64.eq opcode
        assert!(wasm.contains(&OP_I64_EQ));
    }
}
