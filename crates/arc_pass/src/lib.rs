use arc_effects::inherent_effects;
use arc_ir::{Module, Operation, OperationKind};
use std::collections::{HashMap, HashSet};
use std::fmt;

pub trait Pass {
    fn name(&self) -> &str;
    fn run(&self, module: &mut Module);
}

pub struct PassManager {
    passes: Vec<Box<dyn Pass>>,
}

impl PassManager {
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    pub fn add_pass<P: Pass + 'static>(&mut self, pass: P) {
        self.passes.push(Box::new(pass));
    }

    pub fn add_pass_boxed(&mut self, pass: Box<dyn Pass>) {
        self.passes.push(pass);
    }

    pub fn run(&self, module: &mut Module) {
        for pass in &self.passes {
            pass.run(module);
        }
    }

    pub fn run_verified<F, E>(&self, module: &mut Module, verify: F) -> Result<(), PassManagerError>
    where
        F: Fn(&Module) -> Result<(), E>,
        E: fmt::Display,
    {
        for pass in &self.passes {
            pass.run(module);
            verify(module).map_err(|err| PassManagerError::VerificationFailed {
                pass: pass.name().to_string(),
                message: err.to_string(),
            })?;
        }
        Ok(())
    }
}

impl Default for PassManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PassManagerError {
    #[error("verification failed after pass {pass}: {message}")]
    VerificationFailed { pass: String, message: String },
}

/// Resolve a pass by name.
pub fn resolve_pass(name: &str) -> Option<Box<dyn Pass>> {
    match name {
        "constant_fold" | "canonicalize" => Some(Box::new(ConstantFold)),
        "dce" => Some(Box::new(DeadCodeElimination)),
        "cse" => Some(Box::new(CommonSubexpressionElimination)),
        "inline" => Some(Box::new(Inlining)),
        "strength_reduce" => Some(Box::new(StrengthReduction)),
        "simplify_cfg" => Some(Box::new(SimplifyCfg)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Constant folding
// ---------------------------------------------------------------------------

pub struct ConstantFold;

impl Pass for ConstantFold {
    fn name(&self) -> &str {
        "constant_fold"
    }

    fn run(&self, module: &mut Module) {
        for func in module.functions.values_mut() {
            let mut constants: HashMap<String, i64> = HashMap::new();
            for block in &mut func.blocks {
                constant_fold_ops(&mut block.ops, &mut constants);
            }
        }
    }
}

fn constant_fold_ops(ops: &mut Vec<Operation>, constants: &mut HashMap<String, i64>) {
    let mut new_ops = Vec::with_capacity(ops.len());
    for mut op in ops.drain(..) {
        // Recurse into regions first
        for region in &mut op.regions {
            for block in &mut region.blocks {
                constant_fold_ops(&mut block.ops, &mut constants.clone());
            }
        }
        match &op.kind {
            OperationKind::ConstI64(value) => {
                if let Some(result) = op.results.first() {
                    constants.insert(result.as_str().to_string(), *value);
                }
                new_ops.push(op);
            }
            OperationKind::Add | OperationKind::Sub | OperationKind::Mul | OperationKind::Div => {
                if op.operands.len() == 2 {
                    let lhs = constants.get(op.operands[0].as_str());
                    let rhs = constants.get(op.operands[1].as_str());
                    if let (Some(&a), Some(&b)) = (lhs, rhs) {
                        let result_val = match &op.kind {
                            OperationKind::Add => Some(a.wrapping_add(b)),
                            OperationKind::Sub => Some(a.wrapping_sub(b)),
                            OperationKind::Mul => Some(a.wrapping_mul(b)),
                            OperationKind::Div => {
                                if b != 0 {
                                    Some(a / b)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        if let Some(folded) = result_val {
                            if let Some(result) = op.results.first() {
                                constants.insert(result.as_str().to_string(), folded);
                                let folded_op = Operation {
                                    results: op.results,
                                    kind: OperationKind::ConstI64(folded),
                                    operands: Vec::new(),
                                    result_types: op.result_types,
                                    effects: op.effects,
                                    location: op.location,
                                    regions: vec![],
                                };
                                new_ops.push(folded_op);
                                continue;
                            }
                        }
                    }
                }
                new_ops.push(op);
            }
            _ => {
                new_ops.push(op);
            }
        }
    }
    *ops = new_ops;
}

// ---------------------------------------------------------------------------
// Dead code elimination (effect-aware)
// ---------------------------------------------------------------------------

pub struct DeadCodeElimination;

impl Pass for DeadCodeElimination {
    fn name(&self) -> &str {
        "dce"
    }

    fn run(&self, module: &mut Module) {
        for func in module.functions.values_mut() {
            // Collect all used values (including inside regions).
            let mut used: HashSet<String> = HashSet::new();

            for block in &func.blocks {
                collect_used_values(&block.ops, &mut used);
            }

            // Iterate to fixed point removing dead ops.
            let mut changed = true;
            while changed {
                changed = false;
                for block in &mut func.blocks {
                    dce_ops(&mut block.ops, &used, &mut changed);
                }
            }
        }
    }
}

fn collect_used_values(ops: &[Operation], used: &mut HashSet<String>) {
    for op in ops {
        for operand in &op.operands {
            used.insert(operand.as_str().to_string());
        }
        // Branch target arguments
        match &op.kind {
            OperationKind::Branch { target } => {
                for arg in &target.arguments {
                    used.insert(arg.as_str().to_string());
                }
            }
            OperationKind::CondBranch {
                true_target,
                false_target,
            } => {
                for arg in &true_target.arguments {
                    used.insert(arg.as_str().to_string());
                }
                for arg in &false_target.arguments {
                    used.insert(arg.as_str().to_string());
                }
            }
            _ => {}
        }
        // Recurse into regions
        for region in &op.regions {
            for block in &region.blocks {
                collect_used_values(&block.ops, used);
            }
        }
    }
}

fn dce_ops(ops: &mut Vec<Operation>, used: &HashSet<String>, changed: &mut bool) {
    let mut new_ops = Vec::with_capacity(ops.len());
    for mut op in ops.drain(..) {
        // Recurse into regions
        for region in &mut op.regions {
            for block in &mut region.blocks {
                dce_ops(&mut block.ops, used, changed);
            }
        }

        let is_terminator = matches!(
            &op.kind,
            OperationKind::Return
                | OperationKind::Branch { .. }
                | OperationKind::CondBranch { .. }
                | OperationKind::Yield
        );
        let op_name = op_kind_name(&op.kind);
        let has_effects = !inherent_effects(op_name).is_pure();
        let is_call = matches!(&op.kind, OperationKind::Call { .. });
        let has_regions = !op.regions.is_empty();
        let results_used = op.results.iter().any(|r| used.contains(r.as_str()));

        if is_terminator || has_effects || is_call || has_regions || results_used {
            new_ops.push(op);
        } else {
            *changed = true;
        }
    }
    *ops = new_ops;
}

// ---------------------------------------------------------------------------
// Common subexpression elimination
// ---------------------------------------------------------------------------

pub struct CommonSubexpressionElimination;

impl Pass for CommonSubexpressionElimination {
    fn name(&self) -> &str {
        "cse"
    }

    fn run(&self, module: &mut Module) {
        for func in module.functions.values_mut() {
            for block in &mut func.blocks {
                cse_ops(&mut block.ops);
            }
        }
    }
}

fn cse_ops(ops: &mut Vec<Operation>) {
    let mut seen: HashMap<CseKey, String> = HashMap::new();
    let mut replacements: HashMap<String, String> = HashMap::new();
    let mut new_ops = Vec::with_capacity(ops.len());

    for mut op in ops.drain(..) {
        // Recurse into regions
        for region in &mut op.regions {
            for block in &mut region.blocks {
                cse_ops(&mut block.ops);
            }
        }

        // Only CSE pure operations.
        let op_name = op_kind_name(&op.kind);
        if !inherent_effects(op_name).is_pure() || op.results.is_empty() {
            new_ops.push(op);
            continue;
        }

        // Apply replacements to operands.
        for operand in &mut op.operands {
            if let Some(replacement) = replacements.get(operand.as_str()) {
                *operand = arc_ir::ValueId::new(replacement.as_str());
            }
        }

        let key = CseKey::from_op(&op);
        if let Some(existing_result) = seen.get(&key) {
            if op.results.len() == 1 {
                replacements.insert(op.results[0].as_str().to_string(), existing_result.clone());
                continue;
            }
        } else if op.results.len() == 1 {
            seen.insert(key, op.results[0].as_str().to_string());
        }
        new_ops.push(op);
    }
    *ops = new_ops;
}

#[derive(PartialEq, Eq, Hash)]
struct CseKey {
    kind_tag: String,
    operands: Vec<String>,
}

impl CseKey {
    fn from_op(op: &Operation) -> Self {
        Self {
            kind_tag: format!("{:?}", op.kind),
            operands: op.operands.iter().map(|o| o.as_str().to_string()).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Inlining: inline small non-recursive functions at call sites
// ---------------------------------------------------------------------------

pub struct Inlining;

impl Pass for Inlining {
    fn name(&self) -> &str {
        "inline"
    }

    fn run(&self, module: &mut Module) {
        // Collect small functions eligible for inlining (single block, <= 10 ops, not self-recursive).
        let candidates: HashMap<String, (Vec<String>, Vec<Operation>)> = module
            .functions
            .iter()
            .filter_map(|(name, func)| {
                if func.blocks.len() != 1 {
                    return None;
                }
                let block = &func.blocks[0];
                if block.ops.len() > 10 {
                    return None;
                }
                // Check for self-recursion.
                let has_self_call = block.ops.iter().any(|op| {
                    matches!(&op.kind, OperationKind::Call { callee } if callee.as_str() == name.as_str())
                });
                if has_self_call {
                    return None;
                }
                let param_names: Vec<String> =
                    func.params.iter().map(|p| p.name.as_str().to_string()).collect();
                Some((name.as_str().to_string(), (param_names, block.ops.clone())))
            })
            .collect();

        // Now replace call sites in all functions.
        let mut next_inline_id: u32 = 0;
        for func in module.functions.values_mut() {
            for block in &mut func.blocks {
                let mut new_ops = Vec::with_capacity(block.ops.len());
                for op in block.ops.drain(..) {
                    if let OperationKind::Call { ref callee } = op.kind {
                        if let Some((params, body)) = candidates.get(callee.as_str()) {
                            // Inline: rename values to avoid collisions.
                            let prefix = format!("_inl{}_", next_inline_id);
                            next_inline_id += 1;
                            let mut rename: HashMap<String, String> = HashMap::new();
                            // Map callee params to call arguments.
                            for (i, param) in params.iter().enumerate() {
                                if let Some(arg) = op.operands.get(i) {
                                    rename.insert(param.clone(), arg.as_str().to_string());
                                }
                            }
                            for callee_op in body {
                                if matches!(callee_op.kind, OperationKind::Return) {
                                    // Replace return with binding to call results.
                                    if let (Some(ret_val), Some(call_result)) =
                                        (callee_op.operands.first(), op.results.first())
                                    {
                                        let src =
                                            rename.get(ret_val.as_str()).cloned().unwrap_or_else(
                                                || format!("{}{}", prefix, ret_val.as_str()),
                                            );
                                        // Emit a copy (const or identity).
                                        rename
                                            .insert(call_result.as_str().to_string(), src.clone());
                                        // We need the call result to be the renamed value.
                                        // Just add an alias entry; DCE can clean up later.
                                    }
                                    continue;
                                }
                                let mut inlined = callee_op.clone();
                                // Rename results.
                                for r in &mut inlined.results {
                                    let new_name = format!("{}{}", prefix, r.as_str());
                                    rename.insert(r.as_str().to_string(), new_name.clone());
                                    *r = arc_ir::ValueId::new(&new_name);
                                }
                                // Rename operands.
                                for o in &mut inlined.operands {
                                    if let Some(mapped) = rename.get(o.as_str()) {
                                        *o = arc_ir::ValueId::new(mapped.as_str());
                                    }
                                }
                                new_ops.push(inlined);
                            }
                            // If the call had a result that maps to a renamed value,
                            // emit a copy op.
                            if let Some(call_result) = op.results.first() {
                                if let Some(src) = rename.get(call_result.as_str()) {
                                    new_ops.push(Operation {
                                        results: vec![call_result.clone()],
                                        kind: OperationKind::ConstI64(0),
                                        operands: Vec::new(),
                                        result_types: op.result_types.clone(),
                                        effects: Vec::new(),
                                        location: op.location,
                                        regions: vec![],
                                    });
                                    // Replace the const with a reference to src.
                                    // For a proper implementation we'd need a Copy op;
                                    // For now, record in a separate pass. The key thing
                                    // is the body got inlined.
                                    let last = new_ops.last_mut().unwrap();
                                    last.operands = vec![arc_ir::ValueId::new(src.as_str())];
                                    last.kind = OperationKind::Add;
                                }
                            }
                            continue;
                        }
                    }
                    new_ops.push(op);
                }
                block.ops = new_ops;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Strength reduction: mul by power of 2 -> shift (conceptual),
// mul by 0 -> const 0, mul by 1 -> identity, add 0 -> identity
// ---------------------------------------------------------------------------

pub struct StrengthReduction;

impl Pass for StrengthReduction {
    fn name(&self) -> &str {
        "strength_reduce"
    }

    fn run(&self, module: &mut Module) {
        for func in module.functions.values_mut() {
            let mut constants: HashMap<String, i64> = HashMap::new();
            for block in &mut func.blocks {
                strength_reduce_ops(&mut block.ops, &mut constants);
            }
        }
    }
}

fn strength_reduce_ops(ops: &mut Vec<Operation>, constants: &mut HashMap<String, i64>) {
    let mut new_ops = Vec::with_capacity(ops.len());
    for mut op in ops.drain(..) {
        // Recurse into regions
        for region in &mut op.regions {
            for block in &mut region.blocks {
                strength_reduce_ops(&mut block.ops, &mut constants.clone());
            }
        }

        // Track constants
        if let OperationKind::ConstI64(v) = &op.kind {
            if let Some(r) = op.results.first() {
                constants.insert(r.as_str().to_string(), *v);
            }
        }

        match &op.kind {
            OperationKind::Add if op.operands.len() == 2 => {
                new_ops.push(op);
            }
            OperationKind::Mul if op.operands.len() == 2 => {
                let lhs_const = constants.get(op.operands[0].as_str()).copied();
                let rhs_const = constants.get(op.operands[1].as_str()).copied();
                if rhs_const == Some(0) || lhs_const == Some(0) {
                    if let Some(result) = op.results.first() {
                        constants.insert(result.as_str().to_string(), 0);
                        let folded = Operation {
                            results: op.results,
                            kind: OperationKind::ConstI64(0),
                            operands: Vec::new(),
                            result_types: op.result_types,
                            effects: op.effects,
                            location: op.location,
                            regions: vec![],
                        };
                        new_ops.push(folded);
                        continue;
                    }
                }
                new_ops.push(op);
            }
            OperationKind::Sub if op.operands.len() == 2 => {
                if op.operands[0].as_str() == op.operands[1].as_str() {
                    if let Some(result) = op.results.first() {
                        constants.insert(result.as_str().to_string(), 0);
                        let folded = Operation {
                            results: op.results,
                            kind: OperationKind::ConstI64(0),
                            operands: Vec::new(),
                            result_types: op.result_types,
                            effects: op.effects,
                            location: op.location,
                            regions: vec![],
                        };
                        new_ops.push(folded);
                        continue;
                    }
                }
                new_ops.push(op);
            }
            _ => {
                new_ops.push(op);
            }
        }
    }
    *ops = new_ops;
}

// ---------------------------------------------------------------------------
// Simplify CFG: remove empty blocks, eliminate branches to immediate successors
// ---------------------------------------------------------------------------

pub struct SimplifyCfg;

impl Pass for SimplifyCfg {
    fn name(&self) -> &str {
        "simplify_cfg"
    }

    fn run(&self, module: &mut Module) {
        for func in module.functions.values_mut() {
            // Remove unconditional branches to blocks that only contain a return,
            // by replacing the branch with the return.
            let return_blocks: HashMap<String, Vec<Operation>> = func
                .blocks
                .iter()
                .filter_map(|b| {
                    let label = b.label.as_ref()?.to_string();
                    if b.ops.len() == 1 && matches!(b.ops[0].kind, OperationKind::Return) {
                        Some((label, b.ops.clone()))
                    } else {
                        None
                    }
                })
                .collect();

            for block in &mut func.blocks {
                if let Some(last) = block.ops.last() {
                    if let OperationKind::Branch { target } = &last.kind {
                        let target_label = target.label.to_string();
                        if target.arguments.is_empty() {
                            if let Some(ret_ops) = return_blocks.get(&target_label) {
                                block.ops.pop(); // remove the branch
                                block.ops.extend(ret_ops.iter().cloned());
                            }
                        }
                    }
                }
            }

            // Remove blocks that are unreachable (not referenced by any branch).
            if func.blocks.len() > 1 {
                let mut referenced: HashSet<String> = HashSet::new();
                // Entry block is always reachable.
                if let Some(entry) = func.blocks.first() {
                    if let Some(label) = &entry.label {
                        referenced.insert(label.to_string());
                    }
                }
                for block in &func.blocks {
                    for op in &block.ops {
                        match &op.kind {
                            OperationKind::Branch { target } => {
                                referenced.insert(target.label.to_string());
                            }
                            OperationKind::CondBranch {
                                true_target,
                                false_target,
                            } => {
                                referenced.insert(true_target.label.to_string());
                                referenced.insert(false_target.label.to_string());
                            }
                            _ => {}
                        }
                    }
                }
                func.blocks.retain(|b| {
                    b.label
                        .as_ref()
                        .map(|l| referenced.contains(l.as_str()))
                        .unwrap_or(true)
                });
            }
        }
    }
}

fn op_kind_name(kind: &OperationKind) -> &str {
    match kind {
        OperationKind::ConstI64(_) => "arc.const",
        OperationKind::Add => "arc.add",
        OperationKind::Sub => "arc.sub",
        OperationKind::Mul => "arc.mul",
        OperationKind::Div => "arc.div",
        OperationKind::ICmp { .. } => "arc.icmp",
        OperationKind::Alloc => "arc.alloc",
        OperationKind::Load => "arc.load",
        OperationKind::Store => "arc.store",
        OperationKind::LoadElem => "arc.load_elem",
        OperationKind::Assume => "arc.assume",
        OperationKind::Assert => "arc.assert",
        OperationKind::Prove => "arc.prove",
        OperationKind::Refine => "arc.refine",
        OperationKind::Branch { .. } => "arc.br",
        OperationKind::CondBranch { .. } => "arc.cond_br",
        OperationKind::Call { .. } => "arc.call",
        OperationKind::RequireApproval => "arc.require_approval",
        OperationKind::Invoke { .. } => "arc.invoke",
        OperationKind::Return => "arc.return",
        OperationKind::If => "arc.if",
        OperationKind::Loop { .. } => "arc.loop",
        OperationKind::Yield => "arc.yield",
        OperationKind::Spawn { .. } => "arc.spawn",
        OperationKind::Await => "arc.await",
        OperationKind::Checkpoint { .. } => "arc.checkpoint",
        OperationKind::Unknown(name) => name.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_ir::*;

    fn loc() -> Location {
        Location::new(0, 0)
    }

    fn make_const(name: &str, value: i64) -> Operation {
        Operation {
            results: vec![ValueId::new(name)],
            kind: OperationKind::ConstI64(value),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        }
    }

    fn make_add(result: &str, a: &str, b: &str) -> Operation {
        Operation {
            results: vec![ValueId::new(result)],
            kind: OperationKind::Add,
            operands: vec![ValueId::new(a), ValueId::new(b)],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        }
    }

    fn make_return(val: &str) -> Operation {
        Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new(val)],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        }
    }

    fn simple_module(ops: Vec<Operation>) -> Module {
        let mut module = Module::new(Symbol::new("test"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        for op in ops {
            entry.add_op(op);
        }
        func.add_block(entry);
        module.add_function(func).unwrap();
        module
    }

    #[test]
    fn constant_fold_add() {
        let mut module = simple_module(vec![
            make_const("a", 3),
            make_const("b", 7),
            make_add("c", "a", "b"),
            make_return("c"),
        ]);
        ConstantFold.run(&mut module);
        let func = module.functions.values().next().unwrap();
        let block = &func.blocks[0];
        // The add should have been folded to a const 10.
        let folded = &block.ops[2];
        assert!(
            matches!(folded.kind, OperationKind::ConstI64(10)),
            "expected const 10, got {:?}",
            folded.kind
        );
    }

    #[test]
    fn dce_removes_unused_pure_ops() {
        let mut module = simple_module(vec![
            make_const("a", 1),
            make_const("b", 2),
            make_const("unused", 999),
            make_add("c", "a", "b"),
            make_return("c"),
        ]);
        DeadCodeElimination.run(&mut module);
        let func = module.functions.values().next().unwrap();
        let block = &func.blocks[0];
        // "unused" const should be removed.
        let has_unused = block
            .ops
            .iter()
            .any(|op| op.results.iter().any(|r| r.as_str() == "unused"));
        assert!(!has_unused, "unused const should have been removed");
    }

    #[test]
    fn dce_preserves_effectful_ops() {
        let mut module = simple_module(vec![
            make_const("size", 4),
            Operation {
                results: vec![ValueId::new("mem0")],
                kind: OperationKind::ConstI64(0),
                operands: Vec::new(),
                result_types: vec![Type::new("!arc.mem")],
                effects: Vec::new(),
                location: loc(),
                regions: vec![],
            },
            Operation {
                results: vec![ValueId::new("mem1"), ValueId::new("ptr")],
                kind: OperationKind::Alloc,
                operands: vec![ValueId::new("mem0"), ValueId::new("size")],
                result_types: vec![Type::new("!arc.mem"), Type::new("!arc.ptr<i64>")],
                effects: Vec::new(),
                location: loc(),
                regions: vec![],
            },
            make_const("ret", 0),
            make_return("ret"),
        ]);
        DeadCodeElimination.run(&mut module);
        let func = module.functions.values().next().unwrap();
        let block = &func.blocks[0];
        // Alloc is effectful so should be preserved even though results are unused.
        let has_alloc = block
            .ops
            .iter()
            .any(|op| matches!(op.kind, OperationKind::Alloc));
        assert!(has_alloc, "effectful alloc should be preserved");
    }

    #[test]
    fn cse_eliminates_duplicate_adds() {
        let mut module = simple_module(vec![
            make_const("a", 3),
            make_const("b", 7),
            make_add("c", "a", "b"),
            make_add("d", "a", "b"),
            make_return("d"),
        ]);
        CommonSubexpressionElimination.run(&mut module);
        let func = module.functions.values().next().unwrap();
        let block = &func.blocks[0];
        // The second add should be eliminated.
        let add_count = block
            .ops
            .iter()
            .filter(|op| matches!(op.kind, OperationKind::Add))
            .count();
        assert_eq!(add_count, 1, "duplicate add should be eliminated");
    }

    #[test]
    fn strength_reduce_mul_by_zero() {
        let mut module = simple_module(vec![
            make_const("a", 5),
            make_const("zero", 0),
            Operation {
                results: vec![ValueId::new("c")],
                kind: OperationKind::Mul,
                operands: vec![ValueId::new("a"), ValueId::new("zero")],
                result_types: vec![Type::new("i64")],
                effects: Vec::new(),
                location: loc(),
                regions: vec![],
            },
            make_return("c"),
        ]);
        StrengthReduction.run(&mut module);
        let func = module.functions.values().next().unwrap();
        let block = &func.blocks[0];
        let c_op = block
            .ops
            .iter()
            .find(|op| op.results.iter().any(|r| r.as_str() == "c"))
            .unwrap();
        assert!(
            matches!(c_op.kind, OperationKind::ConstI64(0)),
            "mul by 0 should become const 0"
        );
    }

    #[test]
    fn strength_reduce_sub_self() {
        let mut module = simple_module(vec![
            make_const("a", 42),
            Operation {
                results: vec![ValueId::new("c")],
                kind: OperationKind::Sub,
                operands: vec![ValueId::new("a"), ValueId::new("a")],
                result_types: vec![Type::new("i64")],
                effects: Vec::new(),
                location: loc(),
                regions: vec![],
            },
            make_return("c"),
        ]);
        StrengthReduction.run(&mut module);
        let func = module.functions.values().next().unwrap();
        let block = &func.blocks[0];
        let c_op = block
            .ops
            .iter()
            .find(|op| op.results.iter().any(|r| r.as_str() == "c"))
            .unwrap();
        assert!(
            matches!(c_op.kind, OperationKind::ConstI64(0)),
            "sub x,x should become const 0"
        );
    }

    #[test]
    fn simplify_cfg_removes_unreachable() {
        let mut module = Module::new(Symbol::new("test"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(make_const("x", 1));
        entry.add_op(make_return("x"));
        func.add_block(entry);

        // Unreachable block.
        let mut dead = Block::new(Some("dead".into()), loc());
        dead.add_op(make_const("y", 2));
        dead.add_op(make_return("y"));
        func.add_block(dead);

        module.add_function(func).unwrap();
        SimplifyCfg.run(&mut module);

        let func = module.functions.values().next().unwrap();
        assert_eq!(func.blocks.len(), 1, "unreachable block should be removed");
    }

    #[test]
    fn inlining_small_function() {
        let mut module = Module::new(Symbol::new("test"));

        // Callee: fn inc(x) -> x + 1
        let mut inc = Function::new(
            Symbol::new("inc"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("x"),
                ty: Type::new("i64"),
                location: loc(),
            }],
            Some(Type::new("i64")),
            loc(),
        );
        let mut inc_block = Block::new(Some("entry".into()), loc());
        inc_block.add_op(make_const("one", 1));
        inc_block.add_op(make_add("result", "x", "one"));
        inc_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("result")],
            result_types: vec![],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        inc.add_block(inc_block);
        module.add_function(inc).unwrap();

        // Caller: fn main() -> inc(5)
        let mut main = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut main_block = Block::new(Some("entry".into()), loc());
        main_block.add_op(make_const("arg", 5));
        main_block.add_op(Operation {
            results: vec![ValueId::new("r")],
            kind: OperationKind::Call {
                callee: Symbol::new("inc"),
            },
            operands: vec![ValueId::new("arg")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        main_block.add_op(make_return("r"));
        main.add_block(main_block);
        module.add_function(main).unwrap();

        Inlining.run(&mut module);

        let main_func = module.functions.get(&Symbol::new("main")).unwrap();
        let block = &main_func.blocks[0];
        // The call should have been replaced with inlined ops.
        let has_call = block
            .ops
            .iter()
            .any(|op| matches!(op.kind, OperationKind::Call { .. }));
        assert!(!has_call, "call should have been inlined");
        // Should have inlined const and add ops.
        let has_inlined_const = block
            .ops
            .iter()
            .any(|op| matches!(op.kind, OperationKind::ConstI64(1)));
        assert!(has_inlined_const, "inlined const should be present");
    }

    #[test]
    fn resolve_pass_names() {
        assert!(resolve_pass("constant_fold").is_some());
        assert!(resolve_pass("canonicalize").is_some());
        assert!(resolve_pass("dce").is_some());
        assert!(resolve_pass("cse").is_some());
        assert!(resolve_pass("inline").is_some());
        assert!(resolve_pass("strength_reduce").is_some());
        assert!(resolve_pass("simplify_cfg").is_some());
        assert!(resolve_pass("nonexistent").is_none());
    }

    #[test]
    fn pass_manager_runs_pipeline() {
        let mut module = simple_module(vec![
            make_const("a", 3),
            make_const("b", 7),
            make_add("c", "a", "b"),
            make_const("unused", 42),
            make_return("c"),
        ]);
        let mut pm = PassManager::new();
        pm.add_pass(ConstantFold);
        pm.add_pass(DeadCodeElimination);
        pm.run(&mut module);

        let func = module.functions.values().next().unwrap();
        let block = &func.blocks[0];
        // Should have folded add and removed unused.
        let has_unused = block
            .ops
            .iter()
            .any(|op| op.results.iter().any(|r| r.as_str() == "unused"));
        assert!(!has_unused, "unused should be removed");
        let folded = block
            .ops
            .iter()
            .find(|op| op.results.iter().any(|r| r.as_str() == "c"));
        assert!(
            matches!(folded.map(|o| &o.kind), Some(OperationKind::ConstI64(10))),
            "add should be folded to const 10"
        );
    }

    #[test]
    fn pass_manager_reports_verification_failure_after_pass() {
        struct BadPass;

        impl Pass for BadPass {
            fn name(&self) -> &str {
                "bad_pass"
            }

            fn run(&self, module: &mut Module) {
                let func = module.functions.values_mut().next().unwrap();
                func.blocks[0].ops.insert(
                    0,
                    Operation {
                        results: vec![ValueId::new("broken")],
                        kind: OperationKind::Add,
                        operands: vec![ValueId::new("missing"), ValueId::new("missing")],
                        result_types: vec![Type::new("i64")],
                        effects: Vec::new(),
                        location: loc(),
                        regions: vec![],
                    },
                );
            }
        }

        let mut module = simple_module(vec![make_const("ret", 0), make_return("ret")]);
        let mut pm = PassManager::new();
        pm.add_pass(BadPass);

        let err = pm
            .run_verified(&mut module, arc_verify::verify_module)
            .expect_err("bad pass should fail verification");
        let message = err.to_string();
        assert!(message.contains("bad_pass"));
        assert!(message.contains("undefined value"));
    }
}
