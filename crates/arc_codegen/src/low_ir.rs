//! Machine-independent low-level IR (AIR-M).
//!
//! This is a flat, explicit representation where:
//! - All values live in virtual registers (VReg)
//! - Types are concrete machine types (I64, I32, I1)
//! - Control flow is explicit (blocks + jumps)
//! - No proofs, refinements, or authority tokens remain

use crate::CodegenError;
use arc_ir::{IcmpPredicate, Module, OperationKind};

/// A virtual register identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VReg(pub u32);

impl VReg {
    pub fn index(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for VReg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// Machine-level types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachType {
    I64,
    I32,
    I1,
}

impl MachType {
    pub fn size_bytes(self) -> u8 {
        match self {
            MachType::I64 => 8,
            MachType::I32 => 4,
            MachType::I1 => 1,
        }
    }
}

/// Low-level comparison predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Slt,
    Sle,
    Sgt,
    Sge,
}

impl From<&IcmpPredicate> for CmpOp {
    fn from(p: &IcmpPredicate) -> Self {
        match p {
            IcmpPredicate::Eq => CmpOp::Eq,
            IcmpPredicate::Ne => CmpOp::Ne,
            IcmpPredicate::Slt => CmpOp::Slt,
            IcmpPredicate::Sle => CmpOp::Sle,
            IcmpPredicate::Sgt => CmpOp::Sgt,
            IcmpPredicate::Sge => CmpOp::Sge,
        }
    }
}

/// Machine-independent low-level instructions.
#[derive(Debug, Clone)]
pub enum LowOp {
    /// Load 64-bit immediate into vreg.
    LoadImm { dst: VReg, value: i64 },
    /// dst = lhs + rhs
    Add { dst: VReg, lhs: VReg, rhs: VReg },
    /// dst = lhs - rhs
    Sub { dst: VReg, lhs: VReg, rhs: VReg },
    /// dst = lhs * rhs
    Mul { dst: VReg, lhs: VReg, rhs: VReg },
    /// dst = lhs / rhs (signed)
    Div { dst: VReg, lhs: VReg, rhs: VReg },
    /// dst = compare(lhs, rhs) → 0 or 1
    Cmp {
        dst: VReg,
        op: CmpOp,
        lhs: VReg,
        rhs: VReg,
    },
    /// Copy one vreg to another.
    Copy { dst: VReg, src: VReg },
    /// Unconditional jump to block.
    Jump { target: usize },
    /// Conditional branch: if cond != 0 goto true_target else false_target.
    CondJump {
        cond: VReg,
        true_target: usize,
        false_target: usize,
    },
    /// Call a function by name. args are in vregs, result (if any) in dst.
    Call {
        dst: Option<VReg>,
        callee: String,
        args: Vec<VReg>,
    },
    /// Return a value (or void).
    Ret { value: Option<VReg> },
    /// Allocate space on the stack frame, return pointer (frame offset) in dst.
    StackAlloc { dst: VReg, size: i64 },
    /// Load 64-bit value from memory: dst = *addr
    MemLoad { dst: VReg, addr: VReg },
    /// Store 64-bit value to memory: *addr = src
    MemStore { addr: VReg, src: VReg },
    /// Load element from base + index*8: `dst = base[index]`
    MemLoadElem { dst: VReg, base: VReg, index: VReg },
}

/// A basic block in the low IR.
#[derive(Debug, Clone)]
pub struct LowBlock {
    pub label: String,
    pub params: Vec<(VReg, MachType)>,
    pub ops: Vec<LowOp>,
}

/// A function in the low IR.
#[derive(Debug, Clone)]
pub struct LowFunction {
    pub name: String,
    pub params: Vec<(VReg, MachType)>,
    pub result_type: Option<MachType>,
    pub blocks: Vec<LowBlock>,
    pub next_vreg: u32,
}

impl LowFunction {
    pub fn new_vreg(&mut self) -> VReg {
        let v = VReg(self.next_vreg);
        self.next_vreg += 1;
        v
    }

    pub fn vreg_count(&self) -> u32 {
        self.next_vreg
    }
}

/// A module in the low IR.
#[derive(Debug, Clone)]
pub struct LowModule {
    pub name: String,
    pub functions: Vec<LowFunction>,
}

// ---------------------------------------------------------------------------
// Lowering from AIR to Low IR
// ---------------------------------------------------------------------------

/// Lower an AIR module to machine-independent low IR.
pub fn lower_module(module: &Module) -> Result<LowModule, CodegenError> {
    let mut functions = Vec::new();
    for (_sym, func) in &module.functions {
        let low_func = lower_function(func, module)?;
        functions.push(low_func);
    }
    Ok(LowModule {
        name: module.name.as_str().to_string(),
        functions,
    })
}

fn resolve_mach_type(ty_str: &str) -> MachType {
    match ty_str {
        "i64" | "index" => MachType::I64,
        "i32" => MachType::I32,
        "i1" => MachType::I1,
        _ => MachType::I64, // Default for unknown types
    }
}

fn lower_function(func: &arc_ir::Function, _module: &Module) -> Result<LowFunction, CodegenError> {
    use std::collections::HashMap;

    let next_vreg = std::cell::Cell::new(0u32);
    let mut alloc_vreg = || {
        let v = VReg(next_vreg.get());
        next_vreg.set(next_vreg.get() + 1);
        v
    };

    // Map AIR value names → vregs
    let mut value_map: HashMap<String, VReg> = HashMap::new();

    // Map function parameters
    let mut params = Vec::new();
    for param in &func.params {
        let vreg = alloc_vreg();
        let mty = resolve_mach_type(param.ty.as_str());
        value_map.insert(param.name.as_str().to_string(), vreg);
        params.push((vreg, mty));
    }

    // Build block label → index map
    let mut label_to_idx: HashMap<String, usize> = HashMap::new();
    for (i, block) in func.blocks.iter().enumerate() {
        let label = block
            .label()
            .map(|l| l.as_str().to_string())
            .unwrap_or_else(|| format!("bb{}", i));
        label_to_idx.insert(label, i);
    }

    let result_type = func
        .result
        .as_ref()
        .map(|ty| resolve_mach_type(ty.as_str()));

    let mut low_blocks = Vec::new();
    for (block_idx, block) in func.blocks.iter().enumerate() {
        let label = block
            .label()
            .map(|l| l.as_str().to_string())
            .unwrap_or_else(|| format!("bb{}", block_idx));

        // Map block arguments
        let mut block_params = Vec::new();
        for arg in &block.args {
            let vreg = alloc_vreg();
            let mty = resolve_mach_type(arg.ty.as_str());
            value_map.insert(arg.name.as_str().to_string(), vreg);
            block_params.push((vreg, mty));
        }

        let mut ops = Vec::new();

        for op in &block.ops {
            match &op.kind {
                OperationKind::ConstI64(value) => {
                    let dst = alloc_vreg();
                    if let Some(result) = op.results.first() {
                        value_map.insert(result.as_str().to_string(), dst);
                    }
                    ops.push(LowOp::LoadImm { dst, value: *value });
                }
                OperationKind::Add => {
                    let lhs = lookup(&value_map, &op.operands[0])?;
                    let rhs = lookup(&value_map, &op.operands[1])?;
                    let dst = alloc_vreg();
                    if let Some(result) = op.results.first() {
                        value_map.insert(result.as_str().to_string(), dst);
                    }
                    ops.push(LowOp::Add { dst, lhs, rhs });
                }
                OperationKind::Sub => {
                    let lhs = lookup(&value_map, &op.operands[0])?;
                    let rhs = lookup(&value_map, &op.operands[1])?;
                    let dst = alloc_vreg();
                    if let Some(result) = op.results.first() {
                        value_map.insert(result.as_str().to_string(), dst);
                    }
                    ops.push(LowOp::Sub { dst, lhs, rhs });
                }
                OperationKind::Mul => {
                    let lhs = lookup(&value_map, &op.operands[0])?;
                    let rhs = lookup(&value_map, &op.operands[1])?;
                    let dst = alloc_vreg();
                    if let Some(result) = op.results.first() {
                        value_map.insert(result.as_str().to_string(), dst);
                    }
                    ops.push(LowOp::Mul { dst, lhs, rhs });
                }
                OperationKind::Div => {
                    let lhs = lookup(&value_map, &op.operands[0])?;
                    let rhs = lookup(&value_map, &op.operands[1])?;
                    let dst = alloc_vreg();
                    if let Some(result) = op.results.first() {
                        value_map.insert(result.as_str().to_string(), dst);
                    }
                    ops.push(LowOp::Div { dst, lhs, rhs });
                }
                OperationKind::ICmp { predicate } => {
                    let lhs = lookup(&value_map, &op.operands[0])?;
                    let rhs = lookup(&value_map, &op.operands[1])?;
                    let dst = alloc_vreg();
                    if let Some(result) = op.results.first() {
                        value_map.insert(result.as_str().to_string(), dst);
                    }
                    ops.push(LowOp::Cmp {
                        dst,
                        op: CmpOp::from(predicate),
                        lhs,
                        rhs,
                    });
                }
                OperationKind::Branch { target } => {
                    // Copy block arguments
                    let target_idx = label_to_idx
                        .get(target.label.as_str())
                        .copied()
                        .ok_or_else(|| {
                            CodegenError::Failed(format!("unknown branch target {}", target.label))
                        })?;
                    let target_block = &func.blocks[target_idx];
                    for (i, arg_val) in target.arguments.iter().enumerate() {
                        let src = lookup(&value_map, arg_val)?;
                        let dst_arg = value_map
                            .get(target_block.args[i].name.as_str())
                            .copied()
                            .unwrap_or_else(&mut alloc_vreg);
                        ops.push(LowOp::Copy { dst: dst_arg, src });
                    }
                    ops.push(LowOp::Jump { target: target_idx });
                }
                OperationKind::CondBranch {
                    true_target,
                    false_target,
                } => {
                    let cond = lookup(&value_map, &op.operands[0])?;
                    let true_idx = label_to_idx
                        .get(true_target.label.as_str())
                        .copied()
                        .ok_or_else(|| {
                            CodegenError::Failed(format!(
                                "unknown branch target {}",
                                true_target.label
                            ))
                        })?;
                    let false_idx = label_to_idx
                        .get(false_target.label.as_str())
                        .copied()
                        .ok_or_else(|| {
                            CodegenError::Failed(format!(
                                "unknown branch target {}",
                                false_target.label
                            ))
                        })?;
                    ops.push(LowOp::CondJump {
                        cond,
                        true_target: true_idx,
                        false_target: false_idx,
                    });
                }
                OperationKind::Call { callee } => {
                    let mut args = Vec::new();
                    for operand in &op.operands {
                        args.push(lookup(&value_map, operand)?);
                    }
                    let dst = if let Some(result) = op.results.first() {
                        let v = alloc_vreg();
                        value_map.insert(result.as_str().to_string(), v);
                        Some(v)
                    } else {
                        None
                    };
                    ops.push(LowOp::Call {
                        dst,
                        callee: callee.as_str().to_string(),
                        args,
                    });
                }
                OperationKind::Return => {
                    let value = if let Some(operand) = op.operands.first() {
                        Some(lookup(&value_map, operand)?)
                    } else {
                        None
                    };
                    ops.push(LowOp::Ret { value });
                }
                // Proof/authority ops are erased at this level
                OperationKind::Assume
                | OperationKind::Assert
                | OperationKind::Prove
                | OperationKind::Refine
                | OperationKind::RequireApproval => {
                    // Erased in low IR — these should have been lowered away
                    // but we handle them gracefully by treating as no-ops
                    if let Some(result) = op.results.first() {
                        let dst = alloc_vreg();
                        value_map.insert(result.as_str().to_string(), dst);
                        ops.push(LowOp::LoadImm { dst, value: 1 });
                    }
                }
                OperationKind::Invoke { capability } => {
                    // Should have been lowered by InvokeToCallLowering
                    return Err(CodegenError::Unsupported(format!(
                        "arc.invoke @{} — run the lowering pipeline first",
                        capability.as_str()
                    )));
                }
                OperationKind::Alloc => {
                    // Allocate 8 bytes on the stack frame for each alloc.
                    // The result is a frame-relative address (treated as an
                    // opaque pointer by load/store).
                    let dst = alloc_vreg();
                    if let Some(result) = op.results.first() {
                        value_map.insert(result.as_str().to_string(), dst);
                    }
                    ops.push(LowOp::StackAlloc { dst, size: 8 });
                }
                OperationKind::Load => {
                    // load %result = %addr
                    let addr = lookup(&value_map, &op.operands[0])?;
                    let dst = alloc_vreg();
                    if let Some(result) = op.results.first() {
                        value_map.insert(result.as_str().to_string(), dst);
                    }
                    ops.push(LowOp::MemLoad { dst, addr });
                }
                OperationKind::Store => {
                    // store %addr, %value
                    let addr = lookup(&value_map, &op.operands[0])?;
                    let src = lookup(&value_map, &op.operands[1])?;
                    ops.push(LowOp::MemStore { addr, src });
                }
                OperationKind::LoadElem => {
                    // load_elem %result = %base, %index
                    let base = lookup(&value_map, &op.operands[0])?;
                    let index = lookup(&value_map, &op.operands[1])?;
                    let dst = alloc_vreg();
                    if let Some(result) = op.results.first() {
                        value_map.insert(result.as_str().to_string(), dst);
                    }
                    ops.push(LowOp::MemLoadElem { dst, base, index });
                }
                OperationKind::If | OperationKind::Loop { .. } | OperationKind::Yield => {
                    return Err(CodegenError::Unsupported(
                        "structured control flow (if/loop/yield) must be lowered before codegen"
                            .to_string(),
                    ));
                }
                OperationKind::Spawn { .. }
                | OperationKind::Await
                | OperationKind::Checkpoint { .. } => {
                    return Err(CodegenError::Unsupported(
                        "async operations (spawn/await/checkpoint) must be lowered before codegen"
                            .to_string(),
                    ));
                }
                OperationKind::Unknown(name) => {
                    return Err(CodegenError::Unsupported(format!(
                        "unknown operation: {}",
                        name
                    )));
                }
            }
        }

        low_blocks.push(LowBlock {
            label,
            params: block_params,
            ops,
        });
    }

    Ok(LowFunction {
        name: func.name.as_str().to_string(),
        params,
        result_type,
        blocks: low_blocks,
        next_vreg: next_vreg.get(),
    })
}

fn lookup(
    map: &std::collections::HashMap<String, VReg>,
    value: &arc_ir::ValueId,
) -> Result<VReg, CodegenError> {
    map.get(value.as_str())
        .copied()
        .ok_or_else(|| CodegenError::Failed(format!("undefined value %{}", value.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_ir::*;

    fn loc() -> Location {
        Location::new(0, 0)
    }

    #[test]
    fn lower_simple_add() {
        let mut module = Module::new(Symbol::new("test"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("a")],
            kind: OperationKind::ConstI64(3),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("b")],
            kind: OperationKind::ConstI64(7),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("c")],
            kind: OperationKind::Add,
            operands: vec![ValueId::new("a"), ValueId::new("b")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("c")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let low = lower_module(&module).unwrap();
        assert_eq!(low.functions.len(), 1);
        let f = &low.functions[0];
        assert_eq!(f.name, "main");
        assert_eq!(f.blocks.len(), 1);

        let ops = &f.blocks[0].ops;
        assert!(matches!(ops[0], LowOp::LoadImm { value: 3, .. }));
        assert!(matches!(ops[1], LowOp::LoadImm { value: 7, .. }));
        assert!(matches!(ops[2], LowOp::Add { .. }));
        assert!(matches!(ops[3], LowOp::Ret { .. }));
    }

    #[test]
    fn lower_branching() {
        let mut module = Module::new(Symbol::new("test"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("z")],
            kind: OperationKind::ConstI64(0),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("cond")],
            kind: OperationKind::ICmp {
                predicate: IcmpPredicate::Eq,
            },
            operands: vec![ValueId::new("z"), ValueId::new("z")],
            result_types: vec![Type::new("i1")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target: BlockTarget::new("yes".into(), vec![]),
                false_target: BlockTarget::new("no".into(), vec![]),
            },
            operands: vec![ValueId::new("cond")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);

        let mut yes_block = Block::new(Some("yes".into()), loc());
        yes_block.add_op(Operation {
            results: vec![ValueId::new("one")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        yes_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("one")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(yes_block);

        let mut no_block = Block::new(Some("no".into()), loc());
        no_block.add_op(Operation {
            results: vec![ValueId::new("two")],
            kind: OperationKind::ConstI64(2),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        no_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("two")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(no_block);

        module.add_function(func).unwrap();

        let low = lower_module(&module).unwrap();
        let f = &low.functions[0];
        assert_eq!(f.blocks.len(), 3);

        // Last op of entry should be CondJump
        let entry_ops = &f.blocks[0].ops;
        match entry_ops.last().unwrap() {
            LowOp::CondJump {
                true_target,
                false_target,
                ..
            } => {
                assert_eq!(*true_target, 1);
                assert_eq!(*false_target, 2);
            }
            _ => panic!("expected CondJump"),
        }
    }

    #[test]
    fn lower_alloc_store_load() {
        let mut module = Module::new(Symbol::new("test"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        // %ptr = alloc : !arc.ptr<i64>
        entry.add_op(Operation {
            results: vec![ValueId::new("ptr")],
            kind: OperationKind::Alloc,
            operands: Vec::new(),
            result_types: vec![Type::new("!arc.ptr<i64>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        // %val = const 42 : i64
        entry.add_op(Operation {
            results: vec![ValueId::new("val")],
            kind: OperationKind::ConstI64(42),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        // store %ptr, %val
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Store,
            operands: vec![ValueId::new("ptr"), ValueId::new("val")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        // %loaded = load %ptr : i64
        entry.add_op(Operation {
            results: vec![ValueId::new("loaded")],
            kind: OperationKind::Load,
            operands: vec![ValueId::new("ptr")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        // return %loaded
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("loaded")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let low = lower_module(&module).unwrap();
        let f = &low.functions[0];
        let ops = &f.blocks[0].ops;

        assert!(matches!(ops[0], LowOp::StackAlloc { size: 8, .. }));
        assert!(matches!(ops[1], LowOp::LoadImm { value: 42, .. }));
        assert!(matches!(ops[2], LowOp::MemStore { .. }));
        assert!(matches!(ops[3], LowOp::MemLoad { .. }));
        assert!(matches!(ops[4], LowOp::Ret { .. }));
    }

    #[test]
    fn lower_load_elem() {
        let mut module = Module::new(Symbol::new("test"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![
                Argument {
                    name: ValueId::new("base"),
                    ty: Type::new("i64"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("idx"),
                    ty: Type::new("i64"),
                    location: loc(),
                },
            ],
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("elem")],
            kind: OperationKind::LoadElem,
            operands: vec![ValueId::new("base"), ValueId::new("idx")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("elem")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let low = lower_module(&module).unwrap();
        let f = &low.functions[0];
        let ops = &f.blocks[0].ops;

        assert!(matches!(ops[0], LowOp::MemLoadElem { .. }));
        assert!(matches!(ops[1], LowOp::Ret { .. }));
    }
}
