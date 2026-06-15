use arc_ir::{
    Argument, Block, BlockTarget, Function, Location, Module, Operation, OperationKind, Region,
    Symbol, Type, ValueId,
};

/// A refinement record documenting what a lowering preserves.
#[derive(Debug, Clone)]
pub struct Refinement {
    pub name: String,
    pub source_level: String,
    pub target_level: String,
    pub preserved: Vec<String>,
}

impl Refinement {
    pub fn new(name: &str, source: &str, target: &str) -> Self {
        Self {
            name: name.to_string(),
            source_level: source.to_string(),
            target_level: target.to_string(),
            preserved: Vec::new(),
        }
    }

    pub fn preserves(mut self, prop: &str) -> Self {
        self.preserved.push(prop.to_string());
        self
    }

    /// Format as AIR textual representation.
    pub fn format(&self) -> String {
        let mut out = format!("arc.refinement @{} {{\n", self.name);
        out.push_str(&format!("  source = @{}\n", self.source_level));
        out.push_str(&format!("  target = @{}\n", self.target_level));
        if !self.preserved.is_empty() {
            out.push_str("  preserves [\n");
            for prop in &self.preserved {
                out.push_str(&format!("    #arc.prop<{}>,\n", prop));
            }
            out.push_str("  ]\n");
        }
        out.push_str("}\n");
        out
    }
}

/// Trait for a lowering pass that transforms a module from one IR level to another.
pub trait LoweringPass {
    /// Name of this lowering.
    fn name(&self) -> &str;

    /// The source IR level.
    fn source_level(&self) -> &str;

    /// The target IR level.
    fn target_level(&self) -> &str;

    /// Apply the lowering to a module, returning the lowered module and refinement record.
    fn lower(&self, module: &Module) -> Result<(Module, Refinement), LoweringError>;
}

#[derive(Debug, thiserror::Error)]
pub enum LoweringError {
    #[error("lowering error: {0}")]
    Failed(String),
}

/// A pipeline of lowering passes applied in sequence.
pub struct LoweringPipeline {
    passes: Vec<Box<dyn LoweringPass>>,
}

impl LoweringPipeline {
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    pub fn add_pass(&mut self, pass: Box<dyn LoweringPass>) {
        self.passes.push(pass);
    }

    /// Run all lowering passes in order, collecting refinement records.
    pub fn run(&self, module: &Module) -> Result<(Module, Vec<Refinement>), LoweringError> {
        let mut current = module.clone();
        let mut refinements = Vec::new();
        for pass in &self.passes {
            let (lowered, refinement) = pass.lower(&current)?;
            current = lowered;
            refinements.push(refinement);
        }
        Ok((current, refinements))
    }
}

impl Default for LoweringPipeline {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Concrete lowering: Invoke → Call ABI lowering
// ---------------------------------------------------------------------------

/// Lowers `arc.invoke @capability(args)` into a `arc.call @__cap_capability(args)`.
/// This converts external capability invocations into regular function calls that
/// the runtime can dispatch through its ABI.
pub struct InvokeToCallLowering;

impl LoweringPass for InvokeToCallLowering {
    fn name(&self) -> &str {
        "invoke_to_call"
    }

    fn source_level(&self) -> &str {
        "arc-core"
    }

    fn target_level(&self) -> &str {
        "arc-cfg"
    }

    fn lower(&self, module: &Module) -> Result<(Module, Refinement), LoweringError> {
        let mut lowered = Module::new(module.name.clone());

        // Generate stub functions for each capability.
        // Each stub has an entry block that returns a default value (0 for
        // integer results, void otherwise). In a full runtime the linker
        // would replace these stubs with capability-provider trampolines.
        for (cap_name, cap) in &module.capabilities {
            let func_name = format!("__cap_{}", cap_name.as_str());
            let params = cap.inputs.clone();
            let result_ty = cap.outputs.first().map(|o| o.ty.clone());
            let mut stub = Function::new(
                Symbol::new(&func_name),
                Vec::new(),
                params,
                result_ty.clone(),
                cap.location,
            );

            // Build an entry block with a default return
            let mut entry = Block::new(Some("entry".into()), cap.location);
            if let Some(ref _rty) = result_ty {
                // Return a default zero value
                let ret_val = ValueId::new("__ret");
                entry.add_op(Operation {
                    results: vec![ret_val.clone()],
                    kind: OperationKind::ConstI64(0),
                    operands: Vec::new(),
                    result_types: vec![result_ty.clone().unwrap()],
                    effects: Vec::new(),
                    location: cap.location,
                    regions: vec![],
                });
                entry.add_op(Operation {
                    results: Vec::new(),
                    kind: OperationKind::Return,
                    operands: vec![ret_val],
                    result_types: Vec::new(),
                    effects: Vec::new(),
                    location: cap.location,
                    regions: vec![],
                });
            } else {
                // Void return
                entry.add_op(Operation {
                    results: Vec::new(),
                    kind: OperationKind::Return,
                    operands: Vec::new(),
                    result_types: Vec::new(),
                    effects: Vec::new(),
                    location: cap.location,
                    regions: vec![],
                });
            }
            stub.add_block(entry);

            lowered
                .add_function(stub)
                .map_err(|e| LoweringError::Failed(e.to_string()))?;
        }

        // Lower functions
        for (_fname, func) in &module.functions {
            let lowered_func = lower_function_invokes(func, module)?;
            lowered
                .add_function(lowered_func)
                .map_err(|e| LoweringError::Failed(e.to_string()))?;
        }

        let refinement = Refinement::new(
            &format!("lower_{}", self.name()),
            self.source_level(),
            self.target_level(),
        )
        .preserves("same_return_value")
        .preserves("same_effect_trace")
        .preserves("same_authority_requirements");

        Ok((lowered, refinement))
    }
}

fn lower_function_invokes(func: &Function, _module: &Module) -> Result<Function, LoweringError> {
    let mut lowered = Function::new(
        func.name.clone(),
        func.index_params.clone(),
        func.params.clone(),
        func.result.clone(),
        func.location,
    );

    for block in &func.blocks {
        let mut lowered_block = Block::new(block.label.clone(), block.location);
        for arg in &block.args {
            lowered_block.add_arg(arg.clone());
        }

        for op in &block.ops {
            match &op.kind {
                OperationKind::Invoke { capability } => {
                    // Lower arc.invoke @cap(args) → arc.call @__cap_cap(args)
                    let callee_name = format!("__cap_{}", capability.as_str());
                    let lowered_op = Operation {
                        results: op.results.clone(),
                        kind: OperationKind::Call {
                            callee: Symbol::new(&callee_name),
                        },
                        operands: op.operands.clone(),
                        result_types: op.result_types.clone(),
                        effects: op.effects.clone(),
                        location: op.location,
                        regions: vec![],
                    };
                    lowered_block.add_op(lowered_op);
                }
                OperationKind::RequireApproval => {
                    // Erase require_approval to a no-op that produces a constant token
                    // In the lowered form, authority is enforced by the runtime ABI
                    let lowered_op = Operation {
                        results: op.results.clone(),
                        kind: OperationKind::ConstI64(1), // authority token = 1 (granted)
                        operands: Vec::new(),
                        result_types: op.result_types.clone(),
                        effects: Vec::new(),
                        location: op.location,
                        regions: vec![],
                    };
                    lowered_block.add_op(lowered_op);
                }
                _ => {
                    lowered_block.add_op(op.clone());
                }
            }
        }
        lowered.add_block(lowered_block);
    }

    Ok(lowered)
}

// ---------------------------------------------------------------------------
// Concrete lowering: Proof erasure
// ---------------------------------------------------------------------------

/// Erases proof-carrying operations in favor of runtime checks.
/// - `arc.assume` → `arc.assert` (runtime check)
/// - `arc.prove` → `arc.assert` (runtime check)
/// - `arc.refine` → pass-through (identity on the value)
pub struct ProofErasureLowering;

impl LoweringPass for ProofErasureLowering {
    fn name(&self) -> &str {
        "proof_erasure"
    }

    fn source_level(&self) -> &str {
        "arc-core"
    }

    fn target_level(&self) -> &str {
        "arc-cfg"
    }

    fn lower(&self, module: &Module) -> Result<(Module, Refinement), LoweringError> {
        let mut lowered = Module::new(module.name.clone());

        // Copy capabilities as-is
        for (_name, cap) in &module.capabilities {
            lowered
                .add_capability(cap.clone())
                .map_err(|e| LoweringError::Failed(e.to_string()))?;
        }

        for (_fname, func) in &module.functions {
            let lowered_func = lower_function_proofs(func)?;
            lowered
                .add_function(lowered_func)
                .map_err(|e| LoweringError::Failed(e.to_string()))?;
        }

        let refinement = Refinement::new(
            &format!("lower_{}", self.name()),
            self.source_level(),
            self.target_level(),
        )
        .preserves("same_return_value")
        .preserves("same_effect_trace");

        Ok((lowered, refinement))
    }
}

fn lower_function_proofs(func: &Function) -> Result<Function, LoweringError> {
    let mut lowered = Function::new(
        func.name.clone(),
        func.index_params.clone(),
        func.params.clone(),
        func.result.clone(),
        func.location,
    );

    for block in &func.blocks {
        let mut lowered_block = Block::new(block.label.clone(), block.location);
        for arg in &block.args {
            lowered_block.add_arg(arg.clone());
        }

        for op in &block.ops {
            match &op.kind {
                OperationKind::Assume => {
                    // Lower assume to assert (runtime check) — no proof produced
                    let assert_op = Operation {
                        results: Vec::new(),
                        kind: OperationKind::Assert,
                        operands: op.operands.clone(),
                        result_types: op.result_types.clone(),
                        effects: Vec::new(),
                        location: op.location,
                        regions: vec![],
                    };
                    lowered_block.add_op(assert_op);
                    // If assume produced a proof result, define it as a constant 1
                    // so downstream uses don't break
                    if let Some(result_id) = op.results.first() {
                        let const_op = Operation {
                            results: vec![result_id.clone()],
                            kind: OperationKind::ConstI64(1),
                            operands: Vec::new(),
                            result_types: vec![Type::new("i64")],
                            effects: Vec::new(),
                            location: op.location,
                            regions: vec![],
                        };
                        lowered_block.add_op(const_op);
                    }
                }
                OperationKind::Prove => {
                    // Lower prove to assert
                    let assert_op = Operation {
                        results: Vec::new(),
                        kind: OperationKind::Assert,
                        operands: op.operands.clone(),
                        result_types: op.result_types.clone(),
                        effects: Vec::new(),
                        location: op.location,
                        regions: vec![],
                    };
                    lowered_block.add_op(assert_op);
                    if let Some(result_id) = op.results.first() {
                        let const_op = Operation {
                            results: vec![result_id.clone()],
                            kind: OperationKind::ConstI64(1),
                            operands: Vec::new(),
                            result_types: vec![Type::new("i64")],
                            effects: Vec::new(),
                            location: op.location,
                            regions: vec![],
                        };
                        lowered_block.add_op(const_op);
                    }
                }
                OperationKind::Refine => {
                    // Refine becomes identity — just pass the value through
                    if let (Some(result_id), Some(operand)) =
                        (op.results.first(), op.operands.first())
                    {
                        // Emit an add with zero to copy the value with a new name
                        let zero_name = format!("__refine_zero_{}", result_id.as_str());
                        let zero_op = Operation {
                            results: vec![ValueId::new(&zero_name)],
                            kind: OperationKind::ConstI64(0),
                            operands: Vec::new(),
                            result_types: vec![Type::new("i64")],
                            effects: Vec::new(),
                            location: op.location,
                            regions: vec![],
                        };
                        let add_op = Operation {
                            results: vec![result_id.clone()],
                            kind: OperationKind::Add,
                            operands: vec![operand.clone(), ValueId::new(&zero_name)],
                            result_types: vec![Type::new("i64")],
                            effects: Vec::new(),
                            location: op.location,
                            regions: vec![],
                        };
                        lowered_block.add_op(zero_op);
                        lowered_block.add_op(add_op);
                    }
                }
                _ => {
                    lowered_block.add_op(op.clone());
                }
            }
        }
        lowered.add_block(lowered_block);
    }

    Ok(lowered)
}

// ---------------------------------------------------------------------------
// Concrete lowering: Structured control flow → flat CFG
// ---------------------------------------------------------------------------

/// Lowers structured control flow (`air.if`, `arc.loop`, `arc.yield`) into
/// flat CFG with `arc.branch` and `arc.cond_branch`.
///
/// - `If` (condition, then-region, else-region) →
///   `CondBranch` to then_entry / else_entry blocks, both merge into a join block.
///
/// - `Loop` (body-region with Yield) →
///   Header block branches into body, body `Yield` with args → back-edge branch
///   to header, `Yield` without args → branch to exit block.
///
/// - `Yield` is consumed within the If/Loop expansion and should not appear at
///   the top level after lowering.
pub struct StructuredControlFlowLowering {
    counter: std::cell::Cell<usize>,
}

impl StructuredControlFlowLowering {
    pub fn new() -> Self {
        Self {
            counter: std::cell::Cell::new(0),
        }
    }

    fn fresh_label(&self, prefix: &str) -> String {
        let n = self.counter.get();
        self.counter.set(n + 1);
        format!("__{}_{}", prefix, n)
    }
}

impl Default for StructuredControlFlowLowering {
    fn default() -> Self {
        Self::new()
    }
}

impl LoweringPass for StructuredControlFlowLowering {
    fn name(&self) -> &str {
        "structured_cf_to_cfg"
    }

    fn source_level(&self) -> &str {
        "arc-core"
    }

    fn target_level(&self) -> &str {
        "arc-cfg"
    }

    fn lower(&self, module: &Module) -> Result<(Module, Refinement), LoweringError> {
        self.counter.set(0);
        let mut lowered = Module::new(module.name.clone());

        for (_name, cap) in &module.capabilities {
            lowered
                .add_capability(cap.clone())
                .map_err(|e| LoweringError::Failed(e.to_string()))?;
        }

        for (_fname, func) in &module.functions {
            let lowered_func = self.lower_function_scf(func)?;
            lowered
                .add_function(lowered_func)
                .map_err(|e| LoweringError::Failed(e.to_string()))?;
        }

        let refinement = Refinement::new(
            &format!("lower_{}", self.name()),
            self.source_level(),
            self.target_level(),
        )
        .preserves("same_return_value")
        .preserves("same_control_flow_semantics");

        Ok((lowered, refinement))
    }
}

impl StructuredControlFlowLowering {
    fn lower_function_scf(&self, func: &Function) -> Result<Function, LoweringError> {
        let mut lowered = Function::new(
            func.name.clone(),
            func.index_params.clone(),
            func.params.clone(),
            func.result.clone(),
            func.location,
        );

        // We process each block, and If/Loop ops may generate additional blocks.
        // Collect all generated blocks, then add them to the function.
        for block in &func.blocks {
            let expanded = self.expand_block(block)?;
            for b in expanded {
                lowered.add_block(b);
            }
        }

        Ok(lowered)
    }

    /// Expand a single block. Returns 1 block if no structured ops, or multiple
    /// blocks if If/Loop ops are expanded.
    fn expand_block(&self, block: &Block) -> Result<Vec<Block>, LoweringError> {
        let mut result_blocks: Vec<Block> = Vec::new();
        let mut current = Block::new(block.label.clone(), block.location);
        for arg in &block.args {
            current.add_arg(arg.clone());
        }

        for op in &block.ops {
            match &op.kind {
                OperationKind::If => {
                    let taken = std::mem::replace(&mut current, Block::new(None, block.location));
                    let (pre_ops_block, new_blocks) = self.lower_if(op, taken)?;
                    result_blocks.push(pre_ops_block);
                    let merge_idx = new_blocks.len() - 1;
                    for (i, b) in new_blocks.into_iter().enumerate() {
                        if i == merge_idx {
                            current = b;
                        } else {
                            result_blocks.push(b);
                        }
                    }
                }
                OperationKind::Loop { .. } => {
                    let taken = std::mem::replace(&mut current, Block::new(None, block.location));
                    let (pre_ops_block, new_blocks) = self.lower_loop(op, taken)?;
                    result_blocks.push(pre_ops_block);
                    let merge_idx = new_blocks.len() - 1;
                    for (i, b) in new_blocks.into_iter().enumerate() {
                        if i == merge_idx {
                            current = b;
                        } else {
                            result_blocks.push(b);
                        }
                    }
                }
                OperationKind::Yield => {
                    current.add_op(Operation {
                        results: vec![],
                        kind: OperationKind::Return,
                        operands: op.operands.clone(),
                        result_types: op.result_types.clone(),
                        effects: vec![],
                        location: op.location,
                        regions: vec![],
                    });
                }
                _ => {
                    current.add_op(op.clone());
                }
            }
        }

        result_blocks.push(current);
        Ok(result_blocks)
    }

    /// Lower an `If` op. Returns:
    /// - The current block (ending with CondBranch to then/else)
    /// - A vec of [then_block(s)..., else_block(s)..., merge_block]
    fn lower_if(
        &self,
        op: &Operation,
        current: Block,
    ) -> Result<(Block, Vec<Block>), LoweringError> {
        if op.regions.len() < 2 {
            return Err(LoweringError::Failed(
                "If op requires 2 regions (then, else)".to_string(),
            ));
        }
        let cond = op
            .operands
            .first()
            .ok_or_else(|| LoweringError::Failed("If op requires a condition operand".into()))?;

        let then_label = self.fresh_label("if_then");
        let else_label = self.fresh_label("if_else");
        let merge_label = self.fresh_label("if_merge");

        let loc = op.location;

        // End the current block with a CondBranch
        let mut pre_block = current;
        pre_block.add_op(Operation {
            results: vec![],
            kind: OperationKind::CondBranch {
                true_target: BlockTarget::new(then_label.clone().into(), vec![]),
                false_target: BlockTarget::new(else_label.clone().into(), vec![]),
            },
            operands: vec![cond.clone()],
            result_types: vec![],
            effects: vec![],
            location: loc,
            regions: vec![],
        });

        let result_id = op.results.first();
        let mut extra_blocks = Vec::new();

        // Flatten then-region
        let then_blocks =
            self.flatten_region(&op.regions[0], &then_label, &merge_label, result_id, loc)?;
        extra_blocks.extend(then_blocks);

        // Flatten else-region
        let else_blocks =
            self.flatten_region(&op.regions[1], &else_label, &merge_label, result_id, loc)?;
        extra_blocks.extend(else_blocks);

        // Create merge block. If the If produces a result, the merge block takes
        // it as a block argument.
        let mut merge = Block::new(Some(merge_label.into()), loc);
        if let Some(rid) = result_id {
            merge.add_arg(Argument {
                name: rid.clone(),
                ty: op
                    .result_types
                    .first()
                    .cloned()
                    .unwrap_or_else(|| Type::new("i64")),
                location: loc,
            });
        }
        extra_blocks.push(merge);

        Ok((pre_block, extra_blocks))
    }

    /// Lower a `Loop` op. Returns:
    /// - The current block (ending with Branch to the header)
    /// - A vec of [header, body_block(s)..., exit_block]
    fn lower_loop(
        &self,
        op: &Operation,
        current: Block,
    ) -> Result<(Block, Vec<Block>), LoweringError> {
        if op.regions.is_empty() {
            return Err(LoweringError::Failed(
                "Loop op requires 1 region (body)".to_string(),
            ));
        }

        let iter_args = match &op.kind {
            OperationKind::Loop { iter_args } => iter_args.clone(),
            _ => vec![],
        };

        let header_label = self.fresh_label("loop_header");
        let body_label = self.fresh_label("loop_body");
        let exit_label = self.fresh_label("loop_exit");

        let loc = op.location;

        // End the current block with a Branch to the loop header
        let mut pre_block = current;
        pre_block.add_op(Operation {
            results: vec![],
            kind: OperationKind::Branch {
                target: BlockTarget::new(header_label.clone().into(), iter_args.clone()),
            },
            operands: vec![],
            result_types: vec![],
            effects: vec![],
            location: loc,
            regions: vec![],
        });

        let mut extra_blocks = Vec::new();

        // Header block: branches unconditionally into body
        let mut header = Block::new(Some(header_label.clone().into()), loc);
        // Add iter_args as block arguments to the header
        for ia in &iter_args {
            header.add_arg(Argument {
                name: ia.clone(),
                ty: Type::new("i64"),
                location: loc,
            });
        }
        header.add_op(Operation {
            results: vec![],
            kind: OperationKind::Branch {
                target: BlockTarget::new(body_label.clone().into(), vec![]),
            },
            operands: vec![],
            result_types: vec![],
            effects: vec![],
            location: loc,
            regions: vec![],
        });
        extra_blocks.push(header);

        // Flatten body region, but Yield ops need special handling:
        // - Yield with args → branch back to header (continue)
        // - Yield without args → branch to exit (break)
        let body_blocks =
            self.flatten_loop_body(&op.regions[0], &body_label, &header_label, &exit_label, loc)?;
        extra_blocks.extend(body_blocks);

        // Exit block: if loop produces a result, it comes as a block arg
        let mut exit = Block::new(Some(exit_label.into()), loc);
        if let Some(rid) = op.results.first() {
            exit.add_arg(Argument {
                name: rid.clone(),
                ty: op
                    .result_types
                    .first()
                    .cloned()
                    .unwrap_or_else(|| Type::new("i64")),
                location: loc,
            });
        }
        extra_blocks.push(exit);

        Ok((pre_block, extra_blocks))
    }

    /// Flatten a region's blocks, giving the entry block the specified label.
    /// The last block's Yield is converted to a Branch to merge_label,
    /// passing yield values as block arguments.
    fn flatten_region(
        &self,
        region: &Region,
        entry_label: &str,
        merge_label: &str,
        result_id: Option<&ValueId>,
        loc: Location,
    ) -> Result<Vec<Block>, LoweringError> {
        let mut blocks = Vec::new();

        for (i, rblock) in region.blocks.iter().enumerate() {
            let label = if i == 0 {
                entry_label.to_string()
            } else {
                rblock
                    .label
                    .clone()
                    .unwrap_or_else(|| self.fresh_label("region_blk").into())
                    .to_string()
            };

            let mut new_block = Block::new(Some(label.into()), rblock.location);
            for arg in &rblock.args {
                new_block.add_arg(arg.clone());
            }

            for rop in &rblock.ops {
                match &rop.kind {
                    OperationKind::Yield => {
                        // Convert yield to a branch to the merge block
                        let merge_args: Vec<ValueId> = if result_id.is_some() {
                            rop.operands.clone()
                        } else {
                            vec![]
                        };
                        new_block.add_op(Operation {
                            results: vec![],
                            kind: OperationKind::Branch {
                                target: BlockTarget::new(
                                    merge_label.to_string().into(),
                                    merge_args,
                                ),
                            },
                            operands: vec![],
                            result_types: vec![],
                            effects: vec![],
                            location: rop.location,
                            regions: vec![],
                        });
                    }
                    _ => {
                        new_block.add_op(rop.clone());
                    }
                }
            }

            blocks.push(new_block);
        }

        // If the region has no blocks, create a trivial one that branches to merge
        if blocks.is_empty() {
            let mut trivial = Block::new(Some(entry_label.to_string().into()), loc);
            trivial.add_op(Operation {
                results: vec![],
                kind: OperationKind::Branch {
                    target: BlockTarget::new(merge_label.to_string().into(), vec![]),
                },
                operands: vec![],
                result_types: vec![],
                effects: vec![],
                location: loc,
                regions: vec![],
            });
            blocks.push(trivial);
        }

        Ok(blocks)
    }

    /// Flatten a loop body region. Yield ops are converted:
    /// - Yield with operands → Branch back to header (continue with new iter args)
    /// - Yield with no operands → Branch to exit (break)
    fn flatten_loop_body(
        &self,
        region: &Region,
        body_label: &str,
        header_label: &str,
        exit_label: &str,
        loc: Location,
    ) -> Result<Vec<Block>, LoweringError> {
        let mut blocks = Vec::new();

        for (i, rblock) in region.blocks.iter().enumerate() {
            let label = if i == 0 {
                body_label.to_string()
            } else {
                rblock
                    .label
                    .clone()
                    .unwrap_or_else(|| self.fresh_label("loop_blk").into())
                    .to_string()
            };

            let mut new_block = Block::new(Some(label.into()), rblock.location);
            for arg in &rblock.args {
                new_block.add_arg(arg.clone());
            }

            for rop in &rblock.ops {
                match &rop.kind {
                    OperationKind::Yield => {
                        if rop.operands.is_empty() {
                            // Break: branch to exit
                            new_block.add_op(Operation {
                                results: vec![],
                                kind: OperationKind::Branch {
                                    target: BlockTarget::new(exit_label.to_string().into(), vec![]),
                                },
                                operands: vec![],
                                result_types: vec![],
                                effects: vec![],
                                location: rop.location,
                                regions: vec![],
                            });
                        } else {
                            // Continue: branch back to header with updated iter args
                            new_block.add_op(Operation {
                                results: vec![],
                                kind: OperationKind::Branch {
                                    target: BlockTarget::new(
                                        header_label.to_string().into(),
                                        rop.operands.clone(),
                                    ),
                                },
                                operands: vec![],
                                result_types: vec![],
                                effects: vec![],
                                location: rop.location,
                                regions: vec![],
                            });
                        }
                    }
                    _ => {
                        new_block.add_op(rop.clone());
                    }
                }
            }

            blocks.push(new_block);
        }

        if blocks.is_empty() {
            // Empty body: immediately exit
            let mut trivial = Block::new(Some(body_label.to_string().into()), loc);
            trivial.add_op(Operation {
                results: vec![],
                kind: OperationKind::Branch {
                    target: BlockTarget::new(exit_label.to_string().into(), vec![]),
                },
                operands: vec![],
                result_types: vec![],
                effects: vec![],
                location: loc,
                regions: vec![],
            });
            blocks.push(trivial);
        }

        Ok(blocks)
    }
}

// ---------------------------------------------------------------------------
// Concrete lowering: Async → Sequential
// ---------------------------------------------------------------------------

/// Lowers async operations into sequential equivalents:
/// - `Spawn { callee }` → `Call { callee }` (eager sequential execution)
/// - `Await` → identity copy via `Add(operand, 0)`
/// - `Checkpoint { label }` → `ConstI64(0)` (no-op continuation token)
pub struct AsyncLowering;

impl LoweringPass for AsyncLowering {
    fn name(&self) -> &str {
        "async_to_sequential"
    }

    fn source_level(&self) -> &str {
        "arc"
    }

    fn target_level(&self) -> &str {
        "arc.seq"
    }

    fn lower(&self, module: &Module) -> Result<(Module, Refinement), LoweringError> {
        let mut lowered = Module::new(module.name.clone());

        for (_name, cap) in &module.capabilities {
            lowered
                .add_capability(cap.clone())
                .map_err(|e| LoweringError::Failed(e.to_string()))?;
        }

        for (_fname, func) in &module.functions {
            let lowered_func = lower_function_async(func);
            lowered
                .add_function(lowered_func)
                .map_err(|e| LoweringError::Failed(e.to_string()))?;
        }

        let refinement = Refinement::new(
            &format!("lower_{}", self.name()),
            self.source_level(),
            self.target_level(),
        )
        .preserves("same_return_value")
        .preserves("sequential_execution_order");

        Ok((lowered, refinement))
    }
}

fn lower_function_async(func: &Function) -> Function {
    let mut lowered = Function::new(
        func.name.clone(),
        func.index_params.clone(),
        func.params.clone(),
        func.result.clone(),
        func.location,
    );

    for block in &func.blocks {
        let mut lowered_block = Block::new(block.label.clone(), block.location);
        for arg in &block.args {
            lowered_block.add_arg(arg.clone());
        }

        for op in &block.ops {
            match &op.kind {
                OperationKind::Spawn { callee } => {
                    // Spawn → Call (eager sequential execution)
                    let call_op = Operation {
                        results: op.results.clone(),
                        kind: OperationKind::Call {
                            callee: callee.clone(),
                        },
                        operands: op.operands.clone(),
                        result_types: op.result_types.clone(),
                        effects: op.effects.clone(),
                        location: op.location,
                        regions: vec![],
                    };
                    lowered_block.add_op(call_op);
                }
                OperationKind::Await => {
                    // Await → identity copy via Add(operand, 0)
                    if let (Some(result_id), Some(operand)) =
                        (op.results.first(), op.operands.first())
                    {
                        let zero_name = format!("__await_zero_{}", result_id.as_str());
                        let zero_op = Operation {
                            results: vec![ValueId::new(&zero_name)],
                            kind: OperationKind::ConstI64(0),
                            operands: Vec::new(),
                            result_types: vec![Type::new("i64")],
                            effects: Vec::new(),
                            location: op.location,
                            regions: vec![],
                        };
                        let add_op = Operation {
                            results: vec![result_id.clone()],
                            kind: OperationKind::Add,
                            operands: vec![operand.clone(), ValueId::new(&zero_name)],
                            result_types: vec![Type::new("i64")],
                            effects: Vec::new(),
                            location: op.location,
                            regions: vec![],
                        };
                        lowered_block.add_op(zero_op);
                        lowered_block.add_op(add_op);
                    }
                }
                OperationKind::Checkpoint { .. } => {
                    // Checkpoint → ConstI64(0) (no-op continuation token)
                    let const_op = Operation {
                        results: op.results.clone(),
                        kind: OperationKind::ConstI64(0),
                        operands: Vec::new(),
                        result_types: op
                            .result_types
                            .first()
                            .cloned()
                            .map(|t| vec![t])
                            .unwrap_or_else(|| vec![Type::new("i64")]),
                        effects: Vec::new(),
                        location: op.location,
                        regions: vec![],
                    };
                    lowered_block.add_op(const_op);
                }
                _ => {
                    lowered_block.add_op(op.clone());
                }
            }
        }
        lowered.add_block(lowered_block);
    }

    lowered
}

/// Convenience: resolve a lowering pass by name.
pub fn resolve_lowering(name: &str) -> Option<Box<dyn LoweringPass>> {
    match name {
        "invoke_to_call" => Some(Box::new(InvokeToCallLowering)),
        "proof_erasure" => Some(Box::new(ProofErasureLowering)),
        "structured_cf_to_cfg" | "scf_to_cfg" => {
            Some(Box::new(StructuredControlFlowLowering::new()))
        }
        "async_to_sequential" => Some(Box::new(AsyncLowering)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_ir::{
        Argument, Block, Capability, Function, Location, Module, Operation, OperationKind, Symbol,
        Type, ValueId,
    };

    fn loc() -> Location {
        Location::new(0, 0)
    }

    #[test]
    fn invoke_to_call_lowering() {
        let mut module = Module::new(Symbol::new("m"));
        let cap = Capability {
            name: Symbol::new("email.send"),
            inputs: vec![Argument {
                name: ValueId::new("to"),
                ty: Type::new("i64"),
                location: loc(),
            }],
            outputs: vec![Argument {
                name: ValueId::new("status"),
                ty: Type::new("i64"),
                location: loc(),
            }],
            effects: vec!["network".to_string()],
            failures: vec![],
            location: loc(),
        };
        module.add_capability(cap).unwrap();

        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("to")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("status")],
            kind: OperationKind::Invoke {
                capability: Symbol::new("email.send"),
            },
            operands: vec![ValueId::new("to")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("status")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let lowering = InvokeToCallLowering;
        let (lowered, refinement) = lowering.lower(&module).unwrap();

        // Should have stub function + original function
        assert_eq!(lowered.functions.len(), 2);
        assert!(lowered
            .functions
            .contains_key(&Symbol::new("__cap_email.send")));

        // The invoke should be lowered to a call
        let main = lowered.functions.get(&Symbol::new("main")).unwrap();
        let invoke_op = &main.blocks[0].ops[1];
        match &invoke_op.kind {
            OperationKind::Call { callee } => {
                assert_eq!(callee.as_str(), "__cap_email.send");
            }
            other => panic!("expected Call, got {:?}", other),
        }

        // Refinement should document preservation
        assert!(refinement
            .preserved
            .contains(&"same_return_value".to_string()));
        assert!(refinement
            .preserved
            .contains(&"same_effect_trace".to_string()));

        // Verify capability stub has a proper body (entry block with return)
        let stub = lowered
            .functions
            .get(&Symbol::new("__cap_email.send"))
            .unwrap();
        assert_eq!(stub.blocks.len(), 1, "stub should have an entry block");
        let stub_entry = &stub.blocks[0];
        assert!(
            !stub_entry.ops.is_empty(),
            "stub entry should have at least a return op"
        );
        // Last op should be Return
        let last_op = stub_entry.ops.last().unwrap();
        assert!(
            matches!(last_op.kind, OperationKind::Return),
            "stub should end with Return, got {:?}",
            last_op.kind
        );
        // Since the capability has an output, there should be a ConstI64(0) before return
        assert!(
            stub_entry.ops.len() >= 2,
            "stub with output should have const + return"
        );
        assert!(
            matches!(stub_entry.ops[0].kind, OperationKind::ConstI64(0)),
            "stub should produce default 0 value"
        );
    }

    #[test]
    fn capability_stub_void_return() {
        let mut module = Module::new(Symbol::new("m"));
        module
            .add_capability(Capability {
                name: Symbol::new("log.write"),
                inputs: vec![Argument {
                    name: ValueId::new("msg"),
                    ty: Type::new("i64"),
                    location: loc(),
                }],
                outputs: vec![], // void return
                effects: vec!["io".to_string()],
                failures: vec![],
                location: loc(),
            })
            .unwrap();

        // Need at least one function to lower
        let mut func = Function::new(Symbol::new("main"), Vec::new(), Vec::new(), None, loc());
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: Vec::new(),
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let lowering = InvokeToCallLowering;
        let (lowered, _) = lowering.lower(&module).unwrap();

        let stub = lowered
            .functions
            .get(&Symbol::new("__cap_log.write"))
            .unwrap();
        assert_eq!(stub.blocks.len(), 1);
        // Void stub: just a Return with no operands
        let stub_ops = &stub.blocks[0].ops;
        assert_eq!(stub_ops.len(), 1);
        assert!(matches!(stub_ops[0].kind, OperationKind::Return));
        assert!(stub_ops[0].operands.is_empty());
    }

    #[test]
    fn proof_erasure_lowering() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());
        // Create a condition, assume it, then return
        entry.add_op(Operation {
            results: vec![ValueId::new("cond")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i1")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("proof")],
            kind: OperationKind::Assume,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("!arc.proof<true>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("val")],
            kind: OperationKind::ConstI64(42),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("val")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let lowering = ProofErasureLowering;
        let (lowered, refinement) = lowering.lower(&module).unwrap();

        let main = lowered.functions.get(&Symbol::new("main")).unwrap();
        let ops = &main.blocks[0].ops;

        // Assume should become assert + const
        assert!(matches!(ops[1].kind, OperationKind::Assert));
        assert!(matches!(ops[2].kind, OperationKind::ConstI64(1)));
        assert_eq!(ops[2].results[0].as_str(), "proof");

        assert!(refinement
            .preserved
            .contains(&"same_return_value".to_string()));
    }

    #[test]
    fn pipeline_chains_lowerings() {
        let mut module = Module::new(Symbol::new("m"));
        let cap = Capability {
            name: Symbol::new("fs.read"),
            inputs: vec![Argument {
                name: ValueId::new("path"),
                ty: Type::new("i64"),
                location: loc(),
            }],
            outputs: vec![Argument {
                name: ValueId::new("data"),
                ty: Type::new("i64"),
                location: loc(),
            }],
            effects: vec!["filesystem.read".to_string()],
            failures: vec![],
            location: loc(),
        };
        module.add_capability(cap).unwrap();

        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("p")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("d")],
            kind: OperationKind::Invoke {
                capability: Symbol::new("fs.read"),
            },
            operands: vec![ValueId::new("p")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("d")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let mut pipeline = LoweringPipeline::new();
        pipeline.add_pass(Box::new(InvokeToCallLowering));
        // ProofErasure won't change anything here, but tests chaining
        pipeline.add_pass(Box::new(ProofErasureLowering));

        let (lowered, refinements) = pipeline.run(&module).unwrap();
        assert_eq!(refinements.len(), 2);
        assert!(lowered.functions.contains_key(&Symbol::new("main")));
        assert!(lowered
            .functions
            .contains_key(&Symbol::new("__cap_fs.read")));
    }

    #[test]
    fn refinement_format() {
        let r = Refinement::new("lower_invoke", "arc-core", "arc-cfg")
            .preserves("same_return_value")
            .preserves("same_effect_trace");
        let output = r.format();
        assert!(output.contains("arc.refinement @lower_invoke"));
        assert!(output.contains("source = @arc-core"));
        assert!(output.contains("#arc.prop<same_return_value>"));
    }

    #[test]
    fn require_approval_lowered_to_const() {
        let mut module = Module::new(Symbol::new("m"));
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
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("b")],
            kind: OperationKind::ConstI64(2),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("auth")],
            kind: OperationKind::RequireApproval,
            operands: vec![ValueId::new("a"), ValueId::new("b")],
            result_types: vec![Type::new("!arc.auth<test>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("r")],
            kind: OperationKind::ConstI64(42),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("r")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let lowering = InvokeToCallLowering;
        let (lowered, _) = lowering.lower(&module).unwrap();
        let main = lowered.functions.get(&Symbol::new("main")).unwrap();

        // RequireApproval should be lowered to ConstI64(1)
        let auth_op = &main.blocks[0].ops[2];
        assert!(matches!(auth_op.kind, OperationKind::ConstI64(1)));
        assert_eq!(auth_op.results[0].as_str(), "auth");
    }

    // --- Structured control flow lowering tests ---

    /// Helper to build a simple If op with then/else regions.
    fn make_if_op(
        cond: &str,
        result: Option<&str>,
        then_ops: Vec<Operation>,
        else_ops: Vec<Operation>,
    ) -> Operation {
        let mut then_block = Block::new(Some("then".into()), loc());
        for op in then_ops {
            then_block.add_op(op);
        }
        let mut else_block = Block::new(Some("else".into()), loc());
        for op in else_ops {
            else_block.add_op(op);
        }

        let then_region = Region {
            blocks: vec![then_block],
        };
        let else_region = Region {
            blocks: vec![else_block],
        };

        Operation {
            results: result.map(|r| vec![ValueId::new(r)]).unwrap_or_default(),
            kind: OperationKind::If,
            operands: vec![ValueId::new(cond)],
            result_types: result.map(|_| vec![Type::new("i64")]).unwrap_or_default(),
            effects: vec![],
            location: loc(),
            regions: vec![then_region, else_region],
        }
    }

    #[test]
    fn scf_lower_if_produces_cond_branch() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());

        // %cond = arc.const 1 : i1
        entry.add_op(Operation::simple(
            vec![ValueId::new("cond")],
            OperationKind::ConstI64(1),
            vec![],
            vec![Type::new("i1")],
            vec![],
            loc(),
        ));

        // then: yield 10; else: yield 20
        let then_yield = Operation::simple(
            vec![],
            OperationKind::Yield,
            vec![ValueId::new("tv")],
            vec![],
            vec![],
            loc(),
        );
        let else_yield = Operation::simple(
            vec![],
            OperationKind::Yield,
            vec![ValueId::new("ev")],
            vec![],
            vec![],
            loc(),
        );

        let then_const = Operation::simple(
            vec![ValueId::new("tv")],
            OperationKind::ConstI64(10),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        );
        let else_const = Operation::simple(
            vec![ValueId::new("ev")],
            OperationKind::ConstI64(20),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        );

        let if_op = make_if_op(
            "cond",
            Some("r"),
            vec![then_const, then_yield],
            vec![else_const, else_yield],
        );
        entry.add_op(if_op);

        // return %r
        entry.add_op(Operation::simple(
            vec![],
            OperationKind::Return,
            vec![ValueId::new("r")],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        func.add_block(entry);
        module.add_function(func).unwrap();

        let lowering = StructuredControlFlowLowering::new();
        let (lowered, refinement) = lowering.lower(&module).unwrap();

        let main = lowered.functions.get(&Symbol::new("main")).unwrap();

        // Should have: entry (CondBranch), then_block, else_block, merge_block (Return)
        assert!(
            main.blocks.len() >= 4,
            "expected >=4 blocks, got {}",
            main.blocks.len()
        );

        // First block should end with CondBranch
        let entry_term = main.blocks[0].ops.last().unwrap();
        assert!(
            matches!(entry_term.kind, OperationKind::CondBranch { .. }),
            "entry should end with CondBranch, got {:?}",
            entry_term.kind
        );

        // No If ops should remain
        for block in &main.blocks {
            for op in &block.ops {
                assert!(
                    !matches!(op.kind, OperationKind::If),
                    "If op should not remain after lowering"
                );
                assert!(
                    !matches!(op.kind, OperationKind::Yield),
                    "Yield op should not remain after lowering"
                );
            }
        }

        assert!(refinement
            .preserved
            .contains(&"same_return_value".to_string()));
    }

    #[test]
    fn scf_lower_loop_produces_header_and_exit() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());

        // Simple loop that immediately breaks (yield with no args)
        let yield_break = Operation::simple(
            vec![],
            OperationKind::Yield,
            vec![], // no args = break
            vec![],
            vec![],
            loc(),
        );

        let mut body_block = Block::new(Some("body".into()), loc());
        body_block.add_op(yield_break);

        let loop_op = Operation {
            results: vec![],
            kind: OperationKind::Loop { iter_args: vec![] },
            operands: vec![],
            result_types: vec![],
            effects: vec![],
            location: loc(),
            regions: vec![Region {
                blocks: vec![body_block],
            }],
        };
        entry.add_op(loop_op);

        // After the loop, return a constant
        entry.add_op(Operation::simple(
            vec![ValueId::new("r")],
            OperationKind::ConstI64(42),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        entry.add_op(Operation::simple(
            vec![],
            OperationKind::Return,
            vec![ValueId::new("r")],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        func.add_block(entry);
        module.add_function(func).unwrap();

        let lowering = StructuredControlFlowLowering::new();
        let (lowered, _) = lowering.lower(&module).unwrap();
        let main = lowered.functions.get(&Symbol::new("main")).unwrap();

        // Should have: entry (Branch to header), header, body (Branch to exit), exit (const + return)
        assert!(
            main.blocks.len() >= 4,
            "expected >=4 blocks, got {}",
            main.blocks.len()
        );

        // No Loop or Yield ops should remain
        for block in &main.blocks {
            for op in &block.ops {
                assert!(
                    !matches!(op.kind, OperationKind::Loop { .. }),
                    "Loop op should not remain after lowering"
                );
                assert!(
                    !matches!(op.kind, OperationKind::Yield),
                    "Yield op should not remain after lowering"
                );
            }
        }

        // Verify there's at least one Branch targeting the header (back-edge or entry)
        let has_branch_to_header = main.blocks.iter().any(|b| {
            b.ops.iter().any(|op| matches!(&op.kind, OperationKind::Branch { target } if target.label.starts_with("__loop_header")))
        });
        assert!(
            has_branch_to_header,
            "should have a branch to the loop header"
        );
    }

    #[test]
    fn scf_lower_no_structured_ops_is_identity() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation::simple(
            vec![ValueId::new("r")],
            OperationKind::ConstI64(42),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        entry.add_op(Operation::simple(
            vec![],
            OperationKind::Return,
            vec![ValueId::new("r")],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        func.add_block(entry);
        module.add_function(func).unwrap();

        let lowering = StructuredControlFlowLowering::new();
        let (lowered, _) = lowering.lower(&module).unwrap();
        let main = lowered.functions.get(&Symbol::new("main")).unwrap();

        // Should be exactly 1 block with 2 ops, unchanged
        assert_eq!(main.blocks.len(), 1);
        assert_eq!(main.blocks[0].ops.len(), 2);
        assert!(matches!(
            main.blocks[0].ops[0].kind,
            OperationKind::ConstI64(42)
        ));
        assert!(matches!(main.blocks[0].ops[1].kind, OperationKind::Return));
    }

    #[test]
    fn resolve_lowering_by_name() {
        assert!(resolve_lowering("invoke_to_call").is_some());
        assert!(resolve_lowering("proof_erasure").is_some());
        assert!(resolve_lowering("structured_cf_to_cfg").is_some());
        assert!(resolve_lowering("scf_to_cfg").is_some());
        assert!(resolve_lowering("nonexistent").is_none());
    }

    // --- Async lowering tests ---

    #[test]
    fn test_async_lowering_spawn_becomes_call() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("task")],
            kind: OperationKind::Spawn {
                callee: Symbol::new("worker"),
            },
            operands: vec![],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("task")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let lowering = AsyncLowering;
        let (lowered, refinement) = lowering.lower(&module).unwrap();
        let main = lowered.functions.get(&Symbol::new("main")).unwrap();
        let spawn_op = &main.blocks[0].ops[0];
        match &spawn_op.kind {
            OperationKind::Call { callee } => {
                assert_eq!(callee.as_str(), "worker");
            }
            other => panic!("expected Call, got {:?}", other),
        }
        assert_eq!(spawn_op.results[0].as_str(), "task");
        assert!(refinement
            .preserved
            .contains(&"same_return_value".to_string()));
    }

    #[test]
    fn test_async_lowering_await_becomes_identity() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("handle")],
            kind: OperationKind::ConstI64(42),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("val")],
            kind: OperationKind::Await,
            operands: vec![ValueId::new("handle")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("val")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let lowering = AsyncLowering;
        let (lowered, _) = lowering.lower(&module).unwrap();
        let main = lowered.functions.get(&Symbol::new("main")).unwrap();
        let ops = &main.blocks[0].ops;

        // ops[0] = ConstI64(42) for "handle"
        // ops[1] = ConstI64(0)  for "__await_zero_val"
        // ops[2] = Add(handle, __await_zero_val) → "val"
        // ops[3] = Return
        assert_eq!(ops.len(), 4);
        assert!(matches!(ops[1].kind, OperationKind::ConstI64(0)));
        assert!(matches!(ops[2].kind, OperationKind::Add));
        assert_eq!(ops[2].results[0].as_str(), "val");
        assert_eq!(ops[2].operands[0].as_str(), "handle");
    }

    #[test]
    fn test_async_lowering_checkpoint_becomes_const() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("cp")],
            kind: OperationKind::Checkpoint {
                label: "save_point".into(),
            },
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("cp")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let lowering = AsyncLowering;
        let (lowered, _) = lowering.lower(&module).unwrap();
        let main = lowered.functions.get(&Symbol::new("main")).unwrap();
        let cp_op = &main.blocks[0].ops[0];
        assert!(matches!(cp_op.kind, OperationKind::ConstI64(0)));
        assert_eq!(cp_op.results[0].as_str(), "cp");
    }

    #[test]
    fn test_async_lowering_preserves_non_async_ops() {
        let mut module = Module::new(Symbol::new("m"));
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
            kind: OperationKind::ConstI64(5),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("b")],
            kind: OperationKind::ConstI64(10),
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

        let lowering = AsyncLowering;
        let (lowered, _) = lowering.lower(&module).unwrap();
        let main = lowered.functions.get(&Symbol::new("main")).unwrap();
        assert_eq!(main.blocks[0].ops.len(), 4);
        assert!(matches!(
            main.blocks[0].ops[0].kind,
            OperationKind::ConstI64(5)
        ));
        assert!(matches!(
            main.blocks[0].ops[1].kind,
            OperationKind::ConstI64(10)
        ));
        assert!(matches!(main.blocks[0].ops[2].kind, OperationKind::Add));
        assert!(matches!(main.blocks[0].ops[3].kind, OperationKind::Return));
    }

    #[test]
    fn test_async_pipeline_order() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("task")],
            kind: OperationKind::Spawn {
                callee: Symbol::new("worker"),
            },
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("task")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        // Add a worker function so Call resolves
        let mut worker = Function::new(
            Symbol::new("worker"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut wentry = Block::new(Some("entry".into()), loc());
        wentry.add_op(Operation {
            results: vec![ValueId::new("r")],
            kind: OperationKind::ConstI64(42),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        wentry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("r")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        worker.add_block(wentry);
        module.add_function(func).unwrap();
        module.add_function(worker).unwrap();

        let mut pipeline = LoweringPipeline::new();
        pipeline.add_pass(Box::new(StructuredControlFlowLowering::new()));
        pipeline.add_pass(Box::new(AsyncLowering));
        pipeline.add_pass(Box::new(InvokeToCallLowering));
        pipeline.add_pass(Box::new(ProofErasureLowering));

        let (lowered, refinements) = pipeline.run(&module).unwrap();
        assert_eq!(refinements.len(), 4);
        let main = lowered.functions.get(&Symbol::new("main")).unwrap();
        // Spawn should have been lowered to Call
        let first_op = &main.blocks[0].ops[0];
        assert!(
            matches!(&first_op.kind, OperationKind::Call { callee } if callee.as_str() == "worker"),
            "Spawn should be lowered to Call, got {:?}",
            first_op.kind
        );
    }

    #[test]
    fn test_async_resolve_lowering() {
        assert!(resolve_lowering("async_to_sequential").is_some());
    }

    #[test]
    fn scf_pipeline_with_other_passes() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation::simple(
            vec![ValueId::new("cond")],
            OperationKind::ConstI64(1),
            vec![],
            vec![Type::new("i1")],
            vec![],
            loc(),
        ));

        // Simple if with no result, just different consts in each branch
        let then_const = Operation::simple(
            vec![ValueId::new("tv")],
            OperationKind::ConstI64(10),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        );
        let then_yield =
            Operation::simple(vec![], OperationKind::Yield, vec![], vec![], vec![], loc());
        let else_const = Operation::simple(
            vec![ValueId::new("ev")],
            OperationKind::ConstI64(20),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        );
        let else_yield =
            Operation::simple(vec![], OperationKind::Yield, vec![], vec![], vec![], loc());
        let if_op = make_if_op(
            "cond",
            None,
            vec![then_const, then_yield],
            vec![else_const, else_yield],
        );
        entry.add_op(if_op);

        entry.add_op(Operation::simple(
            vec![ValueId::new("r")],
            OperationKind::ConstI64(42),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        entry.add_op(Operation::simple(
            vec![],
            OperationKind::Return,
            vec![ValueId::new("r")],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        func.add_block(entry);
        module.add_function(func).unwrap();

        let mut pipeline = LoweringPipeline::new();
        pipeline.add_pass(Box::new(StructuredControlFlowLowering::new()));
        pipeline.add_pass(Box::new(ProofErasureLowering));

        let (lowered, refinements) = pipeline.run(&module).unwrap();
        assert_eq!(refinements.len(), 2);
        let main = lowered.functions.get(&Symbol::new("main")).unwrap();
        assert!(main.blocks.len() >= 4);
    }
}
