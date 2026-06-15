pub mod trace;

use arc_async::{
    CapturedValue, CheckpointStore, ConcurrencyScope, Continuation, TaskId, TaskResult, TaskState,
};
use arc_ir::{Function, Module, OperationKind, Symbol, Type, ValueId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use thiserror::Error;
use trace::{Trace, TraceEventKind};

#[derive(Debug, Error)]
#[error("{0}")]
pub struct InterpreterError(String);

impl InterpreterError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

pub fn run_main(module: &Module) -> Result<Option<i64>, InterpreterError> {
    let main = module
        .functions
        .get(&Symbol::new("main"))
        .ok_or_else(|| InterpreterError::new("module missing @main function"))?;
    if main.blocks.is_empty() {
        return Err(InterpreterError::new("@main has no blocks"));
    }
    if !main.index_params.is_empty() {
        return Err(InterpreterError::new(
            "interpreter does not support index parameters on @main",
        ));
    }

    let mut args = Vec::new();
    for param in &main.params {
        let value = default_value_for_type(&param.ty).ok_or_else(|| {
            InterpreterError::new(format!(
                "interpreter cannot synthesize argument for parameter {}: {}",
                param.name.as_str(),
                param.ty.as_str()
            ))
        })?;
        args.push(value);
    }
    let mut trace = Trace::new();
    let mut async_rt = AsyncRuntime::new();
    let result = call_function(module, main, args, &mut 0usize, &mut trace, &mut async_rt)?;
    match result {
        Some(ValueData::Int(v)) => Ok(Some(v)),
        Some(_) => Ok(None),
        None => Ok(None),
    }
}

/// Run the @main function and return both the result and execution trace.
pub fn run_main_traced(module: &Module) -> Result<(Option<i64>, Trace), InterpreterError> {
    let main = module
        .functions
        .get(&Symbol::new("main"))
        .ok_or_else(|| InterpreterError::new("module missing @main function"))?;
    if main.blocks.is_empty() {
        return Err(InterpreterError::new("@main has no blocks"));
    }

    let mut args = Vec::new();
    for param in &main.params {
        let value = default_value_for_type(&param.ty).ok_or_else(|| {
            InterpreterError::new(format!(
                "interpreter cannot synthesize argument for parameter {}: {}",
                param.name.as_str(),
                param.ty.as_str()
            ))
        })?;
        args.push(value);
    }
    let mut trace = Trace::new();
    let mut async_rt = AsyncRuntime::new();
    let result = call_function(module, main, args, &mut 0usize, &mut trace, &mut async_rt)?;
    let int_result = match result {
        Some(ValueData::Int(v)) => Some(v),
        _ => None,
    };
    Ok((int_result, trace))
}

fn call_function(
    module: &Module,
    func: &Function,
    arguments: Vec<ValueData>,
    total_steps: &mut usize,
    trace: &mut Trace,
    async_rt: &mut AsyncRuntime,
) -> Result<Option<ValueData>, InterpreterError> {
    if func.blocks.is_empty() {
        return Err(InterpreterError::new(format!(
            "function {} has no blocks",
            func.name
        )));
    }

    let func_name = func.name.as_str();
    trace.record(func_name, TraceEventKind::FunctionEntry);

    let mut values: HashMap<String, ValueData> = HashMap::new();
    let mut proof_facts: HashSet<String> = HashSet::new();
    for (param, value) in func.params.iter().zip(arguments) {
        store_value(&mut values, &mut proof_facts, param.name.as_str(), value);
    }
    let mut current_block = 0usize;
    let mut incoming_args: Vec<ValueData> = Vec::new();

    loop {
        *total_steps += 1;
        if *total_steps > 100_000 {
            return Err(InterpreterError::new(
                "interpreter exceeded step limit (possible infinite loop)",
            ));
        }

        let block = func
            .blocks
            .get(current_block)
            .ok_or_else(|| InterpreterError::new("branch to unknown block"))?;
        if incoming_args.len() != block.args.len() {
            return Err(InterpreterError::new(format!(
                "block '{}' expected {} arguments but received {}",
                block
                    .label()
                    .map(|label| label.as_str())
                    .unwrap_or("<entry>"),
                block.args.len(),
                incoming_args.len()
            )));
        }
        for (arg, value) in block.args.iter().zip(incoming_args.drain(..)) {
            store_value(&mut values, &mut proof_facts, arg.name.as_str(), value);
        }

        let mut advanced_control_flow = false;
        for op in &block.ops {
            match &op.kind {
                OperationKind::ConstI64(value) => {
                    let result = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("const missing result"))?;
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        result.as_str(),
                        ValueData::Int(*value),
                    );
                }
                OperationKind::Add => {
                    let lhs = read_int(&values, &op.operands[0])?;
                    let rhs = read_int(&values, &op.operands[1])?;
                    let result = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("add missing result"))?;
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        result.as_str(),
                        ValueData::Int(lhs + rhs),
                    );
                }
                OperationKind::Sub => {
                    let lhs = read_int(&values, &op.operands[0])?;
                    let rhs = read_int(&values, &op.operands[1])?;
                    let result = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("sub missing result"))?;
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        result.as_str(),
                        ValueData::Int(lhs - rhs),
                    );
                }
                OperationKind::Mul => {
                    let lhs = read_int(&values, &op.operands[0])?;
                    let rhs = read_int(&values, &op.operands[1])?;
                    let result = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("mul missing result"))?;
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        result.as_str(),
                        ValueData::Int(lhs * rhs),
                    );
                }
                OperationKind::Div => {
                    let lhs = read_int(&values, &op.operands[0])?;
                    let rhs = read_int(&values, &op.operands[1])?;
                    if rhs == 0 {
                        return Err(InterpreterError::new("division by zero"));
                    }
                    let result = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("div missing result"))?;
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        result.as_str(),
                        ValueData::Int(lhs / rhs),
                    );
                }
                OperationKind::ICmp { predicate } => {
                    let lhs = read_int(&values, &op.operands[0])?;
                    let rhs = read_int(&values, &op.operands[1])?;
                    let result = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("icmp missing result"))?;
                    let value = match predicate {
                        arc_ir::IcmpPredicate::Eq => (lhs == rhs) as i64,
                        arc_ir::IcmpPredicate::Ne => (lhs != rhs) as i64,
                        arc_ir::IcmpPredicate::Slt => (lhs < rhs) as i64,
                        arc_ir::IcmpPredicate::Sle => (lhs <= rhs) as i64,
                        arc_ir::IcmpPredicate::Sgt => (lhs > rhs) as i64,
                        arc_ir::IcmpPredicate::Sge => (lhs >= rhs) as i64,
                    };
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        result.as_str(),
                        ValueData::Int(value),
                    );
                }
                OperationKind::Assume => {
                    let cond = read_int(&values, &op.operands[0])?;
                    if cond == 0 {
                        return Err(InterpreterError::new("assume violated at runtime"));
                    }
                    let proof_desc = op
                        .result_types
                        .first()
                        .map(|ty| ty.as_str().to_string())
                        .unwrap_or_default();
                    trace.record(
                        func_name,
                        TraceEventKind::ProofChecked {
                            description: format!("assume {}", proof_desc),
                        },
                    );
                    if let Some(result_id) = op.results.first() {
                        let proof_ty = op.result_types.first().map(|ty| ty.as_str().to_string());
                        let proof = ValueData::Proof(ProofValue::new(proof_ty));
                        store_value(&mut values, &mut proof_facts, result_id.as_str(), proof);
                    }
                }
                OperationKind::Assert => {
                    let cond = read_int(&values, &op.operands[0])?;
                    if cond == 0 {
                        trace.record(func_name, TraceEventKind::AssertFailed);
                        return Err(InterpreterError::new("assertion failed at runtime"));
                    }
                    trace.record(func_name, TraceEventKind::AssertPassed);
                }
                OperationKind::Prove => {
                    let cond = read_int(&values, &op.operands[0])?;
                    if cond == 0 {
                        return Err(InterpreterError::new(
                            "unable to prove condition at runtime",
                        ));
                    }
                    if let Some(result_id) = op.results.first() {
                        let proof_ty = op.result_types.first().map(|ty| ty.as_str().to_string());
                        let proof = ValueData::Proof(ProofValue::new(proof_ty));
                        store_value(&mut values, &mut proof_facts, result_id.as_str(), proof);
                    }
                }
                OperationKind::Refine => {
                    let value = read_value(&values, &op.operands[0])?;
                    let result_id = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("refine missing result"))?;
                    store_value(&mut values, &mut proof_facts, result_id.as_str(), value);
                }
                OperationKind::Alloc => {
                    let mem = read_mem(&values, &op.operands[0])?;
                    let size_value = read_int(&values, &op.operands[1])?;
                    if size_value < 0 {
                        return Err(InterpreterError::new("arc.alloc size must be non-negative"));
                    }
                    let alloc_size = size_value as usize;
                    let (new_mem, ptr) = mem.allocate(alloc_size);
                    trace.record(
                        func_name,
                        TraceEventKind::MemoryAlloc {
                            region: ptr.region,
                            size: alloc_size,
                        },
                    );
                    let mem_result = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("alloc missing memory result"))?;
                    let ptr_result = op
                        .results
                        .get(1)
                        .ok_or_else(|| InterpreterError::new("alloc missing pointer result"))?;
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        mem_result.as_str(),
                        ValueData::Mem(new_mem),
                    );
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        ptr_result.as_str(),
                        ValueData::Ptr(ptr),
                    );
                }
                OperationKind::Load => {
                    let mem = read_mem(&values, &op.operands[0])?;
                    let ptr = read_ptr(&values, &op.operands[1])?;
                    let loaded = mem.load(&ptr)?;
                    let mem_result = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("load missing memory result"))?;
                    let value_result = op
                        .results
                        .get(1)
                        .ok_or_else(|| InterpreterError::new("load missing value result"))?;
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        mem_result.as_str(),
                        ValueData::Mem(mem),
                    );
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        value_result.as_str(),
                        ValueData::Int(loaded),
                    );
                }
                OperationKind::Store => {
                    let mem = read_mem(&values, &op.operands[0])?;
                    let ptr = read_ptr(&values, &op.operands[1])?;
                    let value = read_int(&values, &op.operands[2])?;
                    let new_mem = mem.store(&ptr, value)?;
                    let mem_result = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("store missing memory result"))?;
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        mem_result.as_str(),
                        ValueData::Mem(new_mem),
                    );
                }
                OperationKind::Branch { target } => {
                    trace.record(
                        func_name,
                        TraceEventKind::BranchTaken {
                            target: target.label.to_string(),
                        },
                    );
                    let (next_idx, args) = resolve_branch_target(func, target, &values)?;
                    current_block = next_idx;
                    incoming_args = args;
                    advanced_control_flow = true;
                    break;
                }
                OperationKind::CondBranch {
                    true_target,
                    false_target,
                } => {
                    let cond_value = read_int(&values, &op.operands[0])?;
                    let target = if cond_value != 0 {
                        true_target
                    } else {
                        false_target
                    };
                    trace.record(
                        func_name,
                        TraceEventKind::BranchTaken {
                            target: target.label.to_string(),
                        },
                    );
                    let (next_idx, args) = resolve_branch_target(func, target, &values)?;
                    current_block = next_idx;
                    incoming_args = args;
                    advanced_control_flow = true;
                    break;
                }
                OperationKind::LoadElem => {
                    if op.operands.len() > 2 {
                        for proof_operand in &op.operands[2..] {
                            read_proof(&values, proof_operand)?;
                        }
                    } else if proof_facts.is_empty() {
                        return Err(InterpreterError::new(
                            "arc.load_elem requires a proof at runtime",
                        ));
                    }
                    let slice = read_slice(&values, &op.operands[0])?;
                    let index_value = read_int(&values, &op.operands[1])?;
                    if index_value < 0 {
                        return Err(InterpreterError::new(
                            "arc.load_elem index must be non-negative",
                        ));
                    }
                    let index = index_value as usize;
                    let element = slice.element_at(index).ok_or_else(|| {
                        InterpreterError::new("arc.load_elem index out of bounds")
                    })?;
                    let result = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("load_elem missing result"))?;
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        result.as_str(),
                        ValueData::Int(element),
                    );
                }
                OperationKind::RequireApproval => {
                    // Require two operands: the action description and the resource
                    if op.operands.len() < 2 {
                        return Err(InterpreterError::new(
                            "arc.require_approval requires at least two operands",
                        ));
                    }
                    trace.record(func_name, TraceEventKind::ApprovalRequested);
                    // In the interpreter, approval is always granted (fake runtime).
                    // Produce an authority token.
                    let result_id = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("require_approval missing result"))?;
                    let auth_ty = op.result_types.first().map(|ty| ty.as_str().to_string());
                    let auth_token = ValueData::Proof(ProofValue::new(auth_ty));
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        result_id.as_str(),
                        auth_token,
                    );
                }
                OperationKind::Invoke { capability } => {
                    // Verify capability exists in module
                    if !module.capabilities.contains_key(capability) {
                        return Err(InterpreterError::new(format!(
                            "invoke references undefined capability {}",
                            capability
                        )));
                    }
                    trace.record(
                        func_name,
                        TraceEventKind::CapabilityInvoked {
                            capability: capability.as_str().to_string(),
                        },
                    );
                    // In the interpreter, capability invocations are simulated.
                    // Bind any results to default values.
                    for (i, result_id) in op.results.iter().enumerate() {
                        let ty = op.result_types.get(i);
                        let value = match ty.map(|t| t.as_str()) {
                            Some("i64") | Some("i32") => ValueData::Int(0),
                            _ => ValueData::Int(0),
                        };
                        store_value(&mut values, &mut proof_facts, result_id.as_str(), value);
                    }
                }
                OperationKind::Call { callee } => {
                    trace.record(
                        func_name,
                        TraceEventKind::Call {
                            callee: callee.as_str().to_string(),
                        },
                    );
                    let callee_func = module.functions.get(callee).ok_or_else(|| {
                        InterpreterError::new(format!("call to undefined function {}", callee))
                    })?;
                    let mut call_args = Vec::new();
                    for operand in &op.operands {
                        call_args.push(read_value(&values, operand)?);
                    }
                    let call_result = call_function(
                        module,
                        callee_func,
                        call_args,
                        total_steps,
                        trace,
                        async_rt,
                    )?;
                    if let Some(result_id) = op.results.first() {
                        let value = call_result.ok_or_else(|| {
                            InterpreterError::new(format!(
                                "call to {} returned void but result expected",
                                callee
                            ))
                        })?;
                        store_value(&mut values, &mut proof_facts, result_id.as_str(), value);
                    }
                }
                OperationKind::Return => {
                    if let Some(value) = op.operands.first() {
                        let result = read_value(&values, value)?;
                        let display = match &result {
                            ValueData::Int(v) => Some(v.to_string()),
                            _ => None,
                        };
                        trace.record(func_name, TraceEventKind::FunctionReturn { value: display });
                        return Ok(Some(result));
                    } else {
                        trace.record(func_name, TraceEventKind::FunctionReturn { value: None });
                        return Ok(None);
                    }
                }
                OperationKind::If => {
                    // Evaluate condition
                    let cond = read_int(&values, &op.operands[0])?;
                    let region_idx = if cond != 0 { 0 } else { 1 };
                    if region_idx < op.regions.len() {
                        let region = &op.regions[region_idx];
                        if let Some(entry) = region.entry_block() {
                            for region_op in &entry.ops {
                                if let OperationKind::Yield = &region_op.kind {
                                    // Yield values back as results of the if
                                    for (i, result) in op.results.iter().enumerate() {
                                        if i < region_op.operands.len() {
                                            let val = read_value(&values, &region_op.operands[i])?;
                                            store_value(
                                                &mut values,
                                                &mut proof_facts,
                                                result.as_str(),
                                                val,
                                            );
                                        }
                                    }
                                    break;
                                }
                                execute_region_op(
                                    region_op,
                                    &mut values,
                                    &mut proof_facts,
                                    module,
                                    func_name,
                                    total_steps,
                                    trace,
                                    async_rt,
                                )?;
                            }
                        }
                    }
                }
                OperationKind::Loop { iter_args } => {
                    // Initialize iter args from operands
                    let mut loop_vals: Vec<ValueData> = Vec::new();
                    for arg in iter_args.iter() {
                        let val = read_value(&values, arg).unwrap_or(ValueData::Int(0));
                        loop_vals.push(val);
                    }

                    let max_iters = 10000;
                    for _ in 0..max_iters {
                        if op.regions.is_empty() {
                            break;
                        }
                        let region = &op.regions[0];
                        let entry = match region.entry_block() {
                            Some(b) => b,
                            None => break,
                        };

                        // Set up iter arg values in env
                        for (i, arg) in entry.args.iter().enumerate() {
                            if i < loop_vals.len() {
                                store_value(
                                    &mut values,
                                    &mut proof_facts,
                                    arg.name.as_str(),
                                    loop_vals[i].clone(),
                                );
                            }
                        }

                        let mut should_break = false;
                        for region_op in &entry.ops {
                            if let OperationKind::Yield = &region_op.kind {
                                if region_op.operands.is_empty() {
                                    should_break = true;
                                } else {
                                    loop_vals.clear();
                                    for operand in &region_op.operands {
                                        loop_vals.push(read_value(&values, operand)?);
                                    }
                                }
                                break;
                            }
                            execute_region_op(
                                region_op,
                                &mut values,
                                &mut proof_facts,
                                module,
                                func_name,
                                total_steps,
                                trace,
                                async_rt,
                            )?;
                        }
                        if should_break {
                            break;
                        }
                    }

                    // Set results from final loop_vals
                    for (i, result) in op.results.iter().enumerate() {
                        if i < loop_vals.len() {
                            store_value(
                                &mut values,
                                &mut proof_facts,
                                result.as_str(),
                                loop_vals[i].clone(),
                            );
                        }
                    }
                }
                OperationKind::Yield => {
                    // Yield is handled within If/Loop region execution
                    // At top level, treat as a no-op
                }
                OperationKind::Spawn { callee } => {
                    trace.record(
                        func_name,
                        TraceEventKind::TaskSpawned {
                            task: callee.as_str().to_string(),
                        },
                    );
                    let callee_func = module.functions.get(callee).ok_or_else(|| {
                        InterpreterError::new(format!(
                            "spawn references undefined function {}",
                            callee
                        ))
                    })?;
                    let task_id = async_rt.spawn_task();
                    // Eagerly execute the spawned task (simulated concurrency)
                    let mut spawn_args = Vec::new();
                    for operand in &op.operands {
                        spawn_args.push(read_value(&values, operand)?);
                    }
                    let spawn_result = call_function(
                        module,
                        callee_func,
                        spawn_args,
                        total_steps,
                        trace,
                        async_rt,
                    )?;
                    async_rt.complete_task(&task_id, spawn_result);
                    let result_id = op
                        .results
                        .first()
                        .ok_or_else(|| InterpreterError::new("spawn missing result"))?;
                    store_value(
                        &mut values,
                        &mut proof_facts,
                        result_id.as_str(),
                        ValueData::TaskHandle(task_id),
                    );
                }
                OperationKind::Await => {
                    let handle = read_task_handle(&values, &op.operands[0])?;
                    trace.record(
                        func_name,
                        TraceEventKind::TaskAwaited {
                            task: handle.clone(),
                        },
                    );
                    let result = async_rt.await_task(&handle)?;
                    if let Some(result_id) = op.results.first() {
                        let val = result.unwrap_or(ValueData::Int(0));
                        store_value(&mut values, &mut proof_facts, result_id.as_str(), val);
                    }
                }
                OperationKind::Checkpoint { label } => {
                    trace.record(
                        func_name,
                        TraceEventKind::Checkpoint {
                            label: label.to_string(),
                        },
                    );
                    // Build a continuation capturing current integer bindings
                    let mut cont = Continuation::new(
                        TaskId::new(func_name),
                        label.as_str(),
                        "i64",
                        current_block,
                    );
                    for (name, val) in &values {
                        if let ValueData::Int(v) = val {
                            cont.capture(name.clone(), CapturedValue::Int(*v));
                        }
                    }
                    let _ = async_rt.checkpoint_store.save(&cont);
                    if let Some(result_id) = op.results.first() {
                        // Produce a token representing the checkpoint
                        store_value(
                            &mut values,
                            &mut proof_facts,
                            result_id.as_str(),
                            ValueData::Int(0),
                        );
                    }
                }
                OperationKind::Unknown(name) => {
                    return Err(InterpreterError::new(format!(
                        "cannot interpret operation {}",
                        name
                    )));
                }
            }
        }

        if !advanced_control_flow {
            return Err(InterpreterError::new(
                "block ended without branch or return terminator",
            ));
        }
    }
}

fn resolve_branch_target(
    function: &arc_ir::Function,
    target: &arc_ir::BlockTarget,
    values: &HashMap<String, ValueData>,
) -> Result<(usize, Vec<ValueData>), InterpreterError> {
    let label = target.label.as_str();
    let index = function
        .block_index_by_label(label)
        .ok_or_else(|| InterpreterError::new(format!("branch target '{}' not found", label)))?;
    let mut args = Vec::with_capacity(target.arguments.len());
    for value_ref in &target.arguments {
        args.push(read_value(values, value_ref)?);
    }
    Ok((index, args))
}

/// Runtime state for async task management during interpretation.
struct AsyncRuntime {
    scope: ConcurrencyScope,
    task_results: HashMap<String, Option<ValueData>>,
    next_task_id: usize,
    checkpoint_store: CheckpointStore,
}

impl AsyncRuntime {
    fn new() -> Self {
        Self {
            scope: ConcurrencyScope::new("interp"),
            task_results: HashMap::new(),
            next_task_id: 0,
            checkpoint_store: CheckpointStore::new(),
        }
    }

    fn spawn_task(&mut self) -> String {
        let id = format!("task_{}", self.next_task_id);
        self.next_task_id += 1;
        let task_id = TaskId::new(&id);
        self.scope.spawn(task_id);
        id
    }

    fn complete_task(&mut self, id: &str, result: Option<ValueData>) {
        let task_id = TaskId::new(id);
        let task_result = match &result {
            Some(ValueData::Int(v)) => TaskResult::Value(*v),
            _ => TaskResult::Void,
        };
        let _ = self.scope.update(
            &task_id,
            TaskState::Completed {
                result: task_result,
            },
        );
        self.task_results.insert(id.to_string(), result);
    }

    fn await_task(&self, id: &str) -> Result<Option<ValueData>, InterpreterError> {
        self.task_results
            .get(id)
            .cloned()
            .ok_or_else(|| InterpreterError::new(format!("task {} not completed", id)))
    }
}

#[derive(Clone)]
enum ValueData {
    Int(i64),
    Proof(ProofValue),
    Mem(MemoryState),
    Ptr(Pointer),
    Slice(SliceValue),
    TaskHandle(String),
}

#[derive(Clone)]
struct ProofValue {
    ty: Option<String>,
}

impl ProofValue {
    fn new(ty: Option<String>) -> Self {
        Self { ty }
    }

    fn ty(&self) -> Option<&str> {
        self.ty.as_deref()
    }
}

#[derive(Clone)]
struct SliceValue {
    data: Rc<RefCell<Vec<i64>>>,
    offset: usize,
    length: usize,
}

impl SliceValue {
    fn from_vec(elements: Vec<i64>) -> Self {
        let length = elements.len();
        Self {
            data: Rc::new(RefCell::new(elements)),
            offset: 0,
            length,
        }
    }

    fn element_at(&self, index: usize) -> Option<i64> {
        if index >= self.length {
            return None;
        }
        let data = self.data.borrow();
        data.get(self.offset + index).copied()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn len(&self) -> usize {
        self.length
    }

    fn shared(data: Rc<RefCell<Vec<i64>>>, offset: usize, length: usize) -> Self {
        Self {
            data,
            offset,
            length,
        }
    }
}

/// Execute an operation within a region body.
/// Supports all op kinds that can appear inside structured control flow regions.
#[allow(clippy::too_many_arguments)]
fn execute_region_op(
    op: &arc_ir::Operation,
    values: &mut HashMap<String, ValueData>,
    proof_facts: &mut HashSet<String>,
    module: &Module,
    func_name: &str,
    total_steps: &mut usize,
    trace: &mut Trace,
    async_rt: &mut AsyncRuntime,
) -> Result<(), InterpreterError> {
    match &op.kind {
        OperationKind::ConstI64(v) => {
            if let Some(result) = op.results.first() {
                store_value(values, proof_facts, result.as_str(), ValueData::Int(*v));
            }
        }
        OperationKind::Add => {
            let a = read_int(values, &op.operands[0])?;
            let b = read_int(values, &op.operands[1])?;
            if let Some(result) = op.results.first() {
                store_value(
                    values,
                    proof_facts,
                    result.as_str(),
                    ValueData::Int(a.wrapping_add(b)),
                );
            }
        }
        OperationKind::Sub => {
            let a = read_int(values, &op.operands[0])?;
            let b = read_int(values, &op.operands[1])?;
            if let Some(result) = op.results.first() {
                store_value(
                    values,
                    proof_facts,
                    result.as_str(),
                    ValueData::Int(a.wrapping_sub(b)),
                );
            }
        }
        OperationKind::Mul => {
            let a = read_int(values, &op.operands[0])?;
            let b = read_int(values, &op.operands[1])?;
            if let Some(result) = op.results.first() {
                store_value(
                    values,
                    proof_facts,
                    result.as_str(),
                    ValueData::Int(a.wrapping_mul(b)),
                );
            }
        }
        OperationKind::Div => {
            let a = read_int(values, &op.operands[0])?;
            let b = read_int(values, &op.operands[1])?;
            if b == 0 {
                return Err(InterpreterError::new("division by zero in region"));
            }
            if let Some(result) = op.results.first() {
                store_value(values, proof_facts, result.as_str(), ValueData::Int(a / b));
            }
        }
        OperationKind::ICmp { predicate } => {
            let a = read_int(values, &op.operands[0])?;
            let b = read_int(values, &op.operands[1])?;
            let result_val = match predicate {
                arc_ir::IcmpPredicate::Eq => (a == b) as i64,
                arc_ir::IcmpPredicate::Ne => (a != b) as i64,
                arc_ir::IcmpPredicate::Slt => (a < b) as i64,
                arc_ir::IcmpPredicate::Sle => (a <= b) as i64,
                arc_ir::IcmpPredicate::Sgt => (a > b) as i64,
                arc_ir::IcmpPredicate::Sge => (a >= b) as i64,
            };
            if let Some(result) = op.results.first() {
                store_value(
                    values,
                    proof_facts,
                    result.as_str(),
                    ValueData::Int(result_val),
                );
            }
        }
        OperationKind::Assume => {
            let cond = read_int(values, &op.operands[0])?;
            if cond == 0 {
                return Err(InterpreterError::new("assume violated at runtime"));
            }
            let proof_desc = op
                .result_types
                .first()
                .map(|ty| ty.as_str().to_string())
                .unwrap_or_default();
            trace.record(
                func_name,
                TraceEventKind::ProofChecked {
                    description: format!("assume {}", proof_desc),
                },
            );
            if let Some(result_id) = op.results.first() {
                let proof_ty = op.result_types.first().map(|ty| ty.as_str().to_string());
                let proof = ValueData::Proof(ProofValue::new(proof_ty));
                store_value(values, proof_facts, result_id.as_str(), proof);
            }
        }
        OperationKind::Assert => {
            let cond = read_int(values, &op.operands[0])?;
            if cond == 0 {
                trace.record(func_name, TraceEventKind::AssertFailed);
                return Err(InterpreterError::new("assertion failed at runtime"));
            }
            trace.record(func_name, TraceEventKind::AssertPassed);
        }
        OperationKind::Prove => {
            let cond = read_int(values, &op.operands[0])?;
            if cond == 0 {
                return Err(InterpreterError::new(
                    "unable to prove condition at runtime",
                ));
            }
            if let Some(result_id) = op.results.first() {
                let proof_ty = op.result_types.first().map(|ty| ty.as_str().to_string());
                let proof = ValueData::Proof(ProofValue::new(proof_ty));
                store_value(values, proof_facts, result_id.as_str(), proof);
            }
        }
        OperationKind::Refine => {
            let value = read_value(values, &op.operands[0])?;
            let result_id = op
                .results
                .first()
                .ok_or_else(|| InterpreterError::new("refine missing result"))?;
            store_value(values, proof_facts, result_id.as_str(), value);
        }
        OperationKind::Alloc => {
            let mem = read_mem(values, &op.operands[0])?;
            let size_value = read_int(values, &op.operands[1])?;
            if size_value < 0 {
                return Err(InterpreterError::new("arc.alloc size must be non-negative"));
            }
            let (new_mem, ptr) = mem.allocate(size_value as usize);
            trace.record(
                func_name,
                TraceEventKind::MemoryAlloc {
                    region: ptr.region,
                    size: size_value as usize,
                },
            );
            let mem_result = op
                .results
                .first()
                .ok_or_else(|| InterpreterError::new("alloc missing memory result"))?;
            let ptr_result = op
                .results
                .get(1)
                .ok_or_else(|| InterpreterError::new("alloc missing pointer result"))?;
            store_value(
                values,
                proof_facts,
                mem_result.as_str(),
                ValueData::Mem(new_mem),
            );
            store_value(
                values,
                proof_facts,
                ptr_result.as_str(),
                ValueData::Ptr(ptr),
            );
        }
        OperationKind::Load => {
            let mem = read_mem(values, &op.operands[0])?;
            let ptr = read_ptr(values, &op.operands[1])?;
            let loaded = mem.load(&ptr)?;
            let mem_result = op
                .results
                .first()
                .ok_or_else(|| InterpreterError::new("load missing memory result"))?;
            let value_result = op
                .results
                .get(1)
                .ok_or_else(|| InterpreterError::new("load missing value result"))?;
            store_value(
                values,
                proof_facts,
                mem_result.as_str(),
                ValueData::Mem(mem),
            );
            store_value(
                values,
                proof_facts,
                value_result.as_str(),
                ValueData::Int(loaded),
            );
        }
        OperationKind::Store => {
            let mem = read_mem(values, &op.operands[0])?;
            let ptr = read_ptr(values, &op.operands[1])?;
            let value = read_int(values, &op.operands[2])?;
            let new_mem = mem.store(&ptr, value)?;
            let mem_result = op
                .results
                .first()
                .ok_or_else(|| InterpreterError::new("store missing memory result"))?;
            store_value(
                values,
                proof_facts,
                mem_result.as_str(),
                ValueData::Mem(new_mem),
            );
        }
        OperationKind::LoadElem => {
            if op.operands.len() > 2 {
                for proof_operand in &op.operands[2..] {
                    read_proof(values, proof_operand)?;
                }
            } else if proof_facts.is_empty() {
                return Err(InterpreterError::new(
                    "arc.load_elem requires a proof at runtime",
                ));
            }
            let slice = read_slice(values, &op.operands[0])?;
            let index_value = read_int(values, &op.operands[1])?;
            if index_value < 0 {
                return Err(InterpreterError::new(
                    "arc.load_elem index must be non-negative",
                ));
            }
            let element = slice
                .element_at(index_value as usize)
                .ok_or_else(|| InterpreterError::new("arc.load_elem index out of bounds"))?;
            let result = op
                .results
                .first()
                .ok_or_else(|| InterpreterError::new("load_elem missing result"))?;
            store_value(
                values,
                proof_facts,
                result.as_str(),
                ValueData::Int(element),
            );
        }
        OperationKind::Call { callee } => {
            trace.record(
                func_name,
                TraceEventKind::Call {
                    callee: callee.as_str().to_string(),
                },
            );
            let callee_func = module.functions.get(callee).ok_or_else(|| {
                InterpreterError::new(format!("call to undefined function {}", callee))
            })?;
            let mut call_args = Vec::new();
            for operand in &op.operands {
                call_args.push(read_value(values, operand)?);
            }
            let call_result =
                call_function(module, callee_func, call_args, total_steps, trace, async_rt)?;
            if let Some(result_id) = op.results.first() {
                let value = call_result.ok_or_else(|| {
                    InterpreterError::new(format!(
                        "call to {} returned void but result expected",
                        callee
                    ))
                })?;
                store_value(values, proof_facts, result_id.as_str(), value);
            }
        }
        OperationKind::If => {
            // Nested if inside a region
            let cond = read_int(values, &op.operands[0])?;
            let region_idx = if cond != 0 { 0 } else { 1 };
            if region_idx < op.regions.len() {
                let region = &op.regions[region_idx];
                if let Some(entry) = region.entry_block() {
                    for region_op in &entry.ops {
                        if let OperationKind::Yield = &region_op.kind {
                            for (i, result) in op.results.iter().enumerate() {
                                if i < region_op.operands.len() {
                                    let val = read_value(values, &region_op.operands[i])?;
                                    store_value(values, proof_facts, result.as_str(), val);
                                }
                            }
                            break;
                        }
                        execute_region_op(
                            region_op,
                            values,
                            proof_facts,
                            module,
                            func_name,
                            total_steps,
                            trace,
                            async_rt,
                        )?;
                    }
                }
            }
        }
        OperationKind::Loop { iter_args } => {
            // Nested loop inside a region
            let mut loop_vals: Vec<ValueData> = Vec::new();
            for arg in iter_args.iter() {
                let val = read_value(values, arg).unwrap_or(ValueData::Int(0));
                loop_vals.push(val);
            }
            let max_iters = 10000;
            for _ in 0..max_iters {
                if op.regions.is_empty() {
                    break;
                }
                let region = &op.regions[0];
                let entry = match region.entry_block() {
                    Some(b) => b,
                    None => break,
                };
                for (i, arg) in entry.args.iter().enumerate() {
                    if i < loop_vals.len() {
                        store_value(values, proof_facts, arg.name.as_str(), loop_vals[i].clone());
                    }
                }
                let mut should_break = false;
                for region_op in &entry.ops {
                    if let OperationKind::Yield = &region_op.kind {
                        if region_op.operands.is_empty() {
                            should_break = true;
                        } else {
                            loop_vals.clear();
                            for operand in &region_op.operands {
                                loop_vals.push(read_value(values, operand)?);
                            }
                        }
                        break;
                    }
                    execute_region_op(
                        region_op,
                        values,
                        proof_facts,
                        module,
                        func_name,
                        total_steps,
                        trace,
                        async_rt,
                    )?;
                }
                if should_break {
                    break;
                }
            }
            for (i, result) in op.results.iter().enumerate() {
                if i < loop_vals.len() {
                    store_value(values, proof_facts, result.as_str(), loop_vals[i].clone());
                }
            }
        }
        OperationKind::RequireApproval => {
            if op.operands.len() < 2 {
                return Err(InterpreterError::new(
                    "arc.require_approval requires at least two operands",
                ));
            }
            trace.record(func_name, TraceEventKind::ApprovalRequested);
            let result_id = op
                .results
                .first()
                .ok_or_else(|| InterpreterError::new("require_approval missing result"))?;
            let auth_ty = op.result_types.first().map(|ty| ty.as_str().to_string());
            store_value(
                values,
                proof_facts,
                result_id.as_str(),
                ValueData::Proof(ProofValue::new(auth_ty)),
            );
        }
        OperationKind::Invoke { capability } => {
            if !module.capabilities.contains_key(capability) {
                return Err(InterpreterError::new(format!(
                    "invoke references undefined capability {}",
                    capability
                )));
            }
            trace.record(
                func_name,
                TraceEventKind::CapabilityInvoked {
                    capability: capability.as_str().to_string(),
                },
            );
            for result_id in op.results.iter() {
                store_value(values, proof_facts, result_id.as_str(), ValueData::Int(0));
            }
        }
        OperationKind::Spawn { callee } => {
            trace.record(
                func_name,
                TraceEventKind::TaskSpawned {
                    task: callee.as_str().to_string(),
                },
            );
            let callee_func = module.functions.get(callee).ok_or_else(|| {
                InterpreterError::new(format!("spawn references undefined function {}", callee))
            })?;
            let task_id = async_rt.spawn_task();
            let mut spawn_args = Vec::new();
            for operand in &op.operands {
                spawn_args.push(read_value(values, operand)?);
            }
            let spawn_result = call_function(
                module,
                callee_func,
                spawn_args,
                total_steps,
                trace,
                async_rt,
            )?;
            async_rt.complete_task(&task_id, spawn_result);
            let result_id = op
                .results
                .first()
                .ok_or_else(|| InterpreterError::new("spawn missing result"))?;
            store_value(
                values,
                proof_facts,
                result_id.as_str(),
                ValueData::TaskHandle(task_id),
            );
        }
        OperationKind::Await => {
            let handle = read_task_handle(values, &op.operands[0])?;
            trace.record(
                func_name,
                TraceEventKind::TaskAwaited {
                    task: handle.clone(),
                },
            );
            let result = async_rt.await_task(&handle)?;
            if let Some(result_id) = op.results.first() {
                let val = result.unwrap_or(ValueData::Int(0));
                store_value(values, proof_facts, result_id.as_str(), val);
            }
        }
        OperationKind::Checkpoint { label } => {
            trace.record(
                func_name,
                TraceEventKind::Checkpoint {
                    label: label.to_string(),
                },
            );
            if let Some(result_id) = op.results.first() {
                store_value(values, proof_facts, result_id.as_str(), ValueData::Int(0));
            }
        }
        // Yield is handled by the caller (If/Loop), not here
        OperationKind::Yield => {}
        // Branch/CondBranch/Return should not appear inside structured regions
        OperationKind::Branch { .. } | OperationKind::CondBranch { .. } | OperationKind::Return => {
            return Err(InterpreterError::new(
                "branch/return not allowed inside structured region",
            ));
        }
        OperationKind::Unknown(name) => {
            return Err(InterpreterError::new(format!(
                "cannot interpret operation {}",
                name
            )));
        }
    }
    Ok(())
}

fn record_proof(value: &ValueData, proof_facts: &mut HashSet<String>) {
    if let ValueData::Proof(proof) = value {
        if let Some(ty) = proof.ty() {
            proof_facts.insert(ty.to_string());
        }
    }
}

fn store_value(
    values: &mut HashMap<String, ValueData>,
    proof_facts: &mut HashSet<String>,
    name: &str,
    value: ValueData,
) {
    record_proof(&value, proof_facts);
    values.insert(name.to_string(), value);
}

fn read_value(
    values: &HashMap<String, ValueData>,
    value: &ValueId,
) -> Result<ValueData, InterpreterError> {
    values
        .get(value.as_str())
        .cloned()
        .ok_or_else(|| InterpreterError::new(format!("value {} not defined", value)))
}

fn read_int(values: &HashMap<String, ValueData>, value: &ValueId) -> Result<i64, InterpreterError> {
    match read_value(values, value)? {
        ValueData::Int(v) => Ok(v),
        _ => Err(InterpreterError::new(format!(
            "expected integer value for {}",
            value
        ))),
    }
}

#[derive(Clone)]
struct MemoryState {
    next_region: usize,
    regions: HashMap<usize, Region>,
}

#[derive(Clone)]
struct Region {
    data: Rc<RefCell<Vec<i64>>>,
}

#[derive(Clone)]
struct Pointer {
    region: usize,
    offset: usize,
}

impl MemoryState {
    fn new() -> Self {
        Self {
            next_region: 0,
            regions: HashMap::new(),
        }
    }

    fn allocate(&self, size: usize) -> (Self, Pointer) {
        let mut new_state = self.clone();
        let region_id = new_state.next_region;
        new_state.next_region += 1;
        let data = Rc::new(RefCell::new(vec![0; size]));
        new_state
            .regions
            .insert(region_id, Region { data: data.clone() });
        (
            new_state,
            Pointer {
                region: region_id,
                offset: 0,
            },
        )
    }

    fn load(&self, ptr: &Pointer) -> Result<i64, InterpreterError> {
        let region = self
            .regions
            .get(&ptr.region)
            .ok_or_else(|| InterpreterError::new("pointer references unknown allocation"))?;
        let data = region.data.borrow();
        data.get(ptr.offset)
            .copied()
            .ok_or_else(|| InterpreterError::new("pointer out of bounds"))
    }

    fn store(&self, ptr: &Pointer, value: i64) -> Result<Self, InterpreterError> {
        let mut new_state = self.clone();
        let region = new_state
            .regions
            .get_mut(&ptr.region)
            .ok_or_else(|| InterpreterError::new("pointer references unknown allocation"))?;
        let mut data = region.data.borrow_mut();
        if ptr.offset >= data.len() {
            return Err(InterpreterError::new("pointer out of bounds"));
        }
        data[ptr.offset] = value;
        drop(data);
        Ok(new_state)
    }

    #[allow(dead_code)]
    fn slice(&self, ptr: &Pointer, length: usize) -> Result<SliceValue, InterpreterError> {
        let region = self
            .regions
            .get(&ptr.region)
            .ok_or_else(|| InterpreterError::new("pointer references unknown allocation"))?;
        {
            let data = region.data.borrow();
            if ptr.offset.saturating_add(length) > data.len() {
                return Err(InterpreterError::new(
                    "slice extends beyond allocation bounds",
                ));
            }
        }
        Ok(SliceValue::shared(region.data.clone(), ptr.offset, length))
    }
}

fn read_mem(
    values: &HashMap<String, ValueData>,
    value: &ValueId,
) -> Result<MemoryState, InterpreterError> {
    match read_value(values, value)? {
        ValueData::Mem(mem) => Ok(mem),
        _ => Err(InterpreterError::new(format!(
            "expected memory value for {}",
            value
        ))),
    }
}

fn read_ptr(
    values: &HashMap<String, ValueData>,
    value: &ValueId,
) -> Result<Pointer, InterpreterError> {
    match read_value(values, value)? {
        ValueData::Ptr(ptr) => Ok(ptr),
        _ => Err(InterpreterError::new(format!(
            "expected pointer value for {}",
            value
        ))),
    }
}

fn read_proof(
    values: &HashMap<String, ValueData>,
    value: &ValueId,
) -> Result<ProofValue, InterpreterError> {
    match read_value(values, value)? {
        ValueData::Proof(proof) => Ok(proof),
        _ => Err(InterpreterError::new(format!(
            "expected proof value for {}",
            value
        ))),
    }
}

fn read_slice(
    values: &HashMap<String, ValueData>,
    value: &ValueId,
) -> Result<SliceValue, InterpreterError> {
    match read_value(values, value)? {
        ValueData::Slice(slice) => Ok(slice),
        _ => Err(InterpreterError::new(format!(
            "expected slice value for {}",
            value
        ))),
    }
}

fn read_task_handle(
    values: &HashMap<String, ValueData>,
    value: &ValueId,
) -> Result<String, InterpreterError> {
    match read_value(values, value)? {
        ValueData::TaskHandle(id) => Ok(id),
        _ => Err(InterpreterError::new(format!(
            "expected task handle for {}",
            value
        ))),
    }
}

fn default_value_for_type(ty: &Type) -> Option<ValueData> {
    match ty.as_str() {
        "!arc.mem" => Some(ValueData::Mem(MemoryState::new())),
        "index" => Some(ValueData::Int(0)),
        "i64" => Some(ValueData::Int(0)),
        _ if ty.as_str().starts_with("!arc.slice<") => {
            Some(ValueData::Slice(SliceValue::from_vec(vec![0, 1, 2, 3])))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_ir::{
        Argument, Block, BlockTarget, Function, IcmpPredicate, Location, Module, Operation,
        OperationKind, Symbol, Type, ValueId,
    };

    fn loc() -> Location {
        Location::new(0, 0)
    }

    #[test]
    fn run_branching_program() {
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
            results: vec![ValueId::new("zero")],
            kind: OperationKind::ConstI64(0),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("one")],
            kind: OperationKind::ConstI64(1),
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
            operands: vec![ValueId::new("zero"), ValueId::new("one")],
            result_types: vec![Type::new("i1")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target: BlockTarget::new("then".into(), vec![]),
                false_target: BlockTarget::new("else".into(), vec![]),
            },
            operands: vec![ValueId::new("cond")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_op(Operation {
            results: vec![ValueId::new("value")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        then_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("value")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut else_block = Block::new(Some("else".into()), loc());
        else_block.add_op(Operation {
            results: vec![ValueId::new("value2")],
            kind: OperationKind::ConstI64(2),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        else_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("value2")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        func.add_block(entry);
        func.add_block(then_block);
        func.add_block(else_block);
        module.add_function(func).unwrap();

        let result = run_main(&module).expect("interpreter succeeded");
        assert_eq!(result, Some(2));
    }

    #[test]
    fn load_elem_reads_element() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("xs"),
                ty: Type::new("!arc.slice<i32, 4>"),
                location: loc(),
            }],
            Some(Type::new("i32")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("idx")],
            kind: OperationKind::ConstI64(2),
            operands: Vec::new(),
            result_types: vec![Type::new("index")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("zero")],
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
            operands: vec![ValueId::new("zero"), ValueId::new("zero")],
            result_types: vec![Type::new("i1")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("pf")],
            kind: OperationKind::Assume,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("!arc.proof<true>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("value")],
            kind: OperationKind::LoadElem,
            operands: vec![ValueId::new("xs"), ValueId::new("idx"), ValueId::new("pf")],
            result_types: vec![Type::new("i32")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("value")],
            result_types: vec![Type::new("i32")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let result = run_main(&module).expect("interpreter succeeded");
        // Default slice contains [0,1,2,3], so index 2 should read value 2.
        assert_eq!(result, Some(2));
    }

    #[test]
    fn load_elem_out_of_bounds_errors() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("xs"),
                ty: Type::new("!arc.slice<i32, 4>"),
                location: loc(),
            }],
            Some(Type::new("i32")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("idx")],
            kind: OperationKind::ConstI64(10),
            operands: Vec::new(),
            result_types: vec![Type::new("index")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("zero")],
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
            operands: vec![ValueId::new("zero"), ValueId::new("zero")],
            result_types: vec![Type::new("i1")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("pf")],
            kind: OperationKind::Assume,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("!arc.proof<true>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("value")],
            kind: OperationKind::LoadElem,
            operands: vec![ValueId::new("xs"), ValueId::new("idx"), ValueId::new("pf")],
            result_types: vec![Type::new("i32")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("value")],
            result_types: vec![Type::new("i32")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let err = run_main(&module).expect_err("load_elem should reject out-of-bounds index");
        let message = format!("{err}");
        assert!(
            message.contains("arc.load_elem index out of bounds"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn load_elem_without_proof_errors() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("xs"),
                ty: Type::new("!arc.slice<i32, 4>"),
                location: loc(),
            }],
            Some(Type::new("i32")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("idx")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("index")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("value")],
            kind: OperationKind::LoadElem,
            operands: vec![ValueId::new("xs"), ValueId::new("idx")],
            result_types: vec![Type::new("i32")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("value")],
            result_types: vec![Type::new("i32")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let err = run_main(&module).expect_err("load_elem without proof should fail at runtime");
        let message = format!("{err}");
        assert!(
            message.contains("requires a proof"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn slice_reflects_memory_updates() {
        let mem = MemoryState::new();
        let (mem1, ptr) = mem.allocate(3);
        let slice = mem1.slice(&ptr, 3).expect("slice creation");
        assert_eq!(slice.len(), 3);
        assert_eq!(slice.element_at(0), Some(0));

        let mem2 = mem1.store(&ptr, 5).expect("store into slice base");
        // slice should observe updated value because it shares underlying storage.
        assert_eq!(slice.element_at(0), Some(5));

        let ptr_end = Pointer {
            region: ptr.region,
            offset: 2,
        };
        let _mem3 = mem2.store(&ptr_end, 9).expect("store at tail element");
        assert_eq!(slice.element_at(2), Some(9));
    }

    #[test]
    fn spawn_and_await_task() {
        // Build a module with a worker function and a main that spawns+awaits it
        let mut module = Module::new(Symbol::new("m"));

        // worker() -> i64 { return 42 }
        let mut worker = Function::new(
            Symbol::new("worker"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut worker_entry = Block::new(Some("entry".into()), loc());
        worker_entry.add_op(Operation {
            results: vec![ValueId::new("v")],
            kind: OperationKind::ConstI64(42),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        worker_entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("v")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        worker.add_block(worker_entry);
        module.add_function(worker).unwrap();

        // main() -> i64 { h = spawn @worker; r = await h; return r }
        let mut main = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut main_entry = Block::new(Some("entry".into()), loc());
        main_entry.add_op(Operation {
            results: vec![ValueId::new("h")],
            kind: OperationKind::Spawn {
                callee: Symbol::new("worker"),
            },
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        main_entry.add_op(Operation {
            results: vec![ValueId::new("r")],
            kind: OperationKind::Await,
            operands: vec![ValueId::new("h")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        main_entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("r")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        main.add_block(main_entry);
        module.add_function(main).unwrap();

        let result = run_main(&module).expect("spawn+await should succeed");
        assert_eq!(result, Some(42));
    }

    #[test]
    fn spawn_await_traced() {
        let mut module = Module::new(Symbol::new("m"));

        let mut worker = Function::new(
            Symbol::new("worker"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut we = Block::new(Some("entry".into()), loc());
        we.add_op(Operation {
            results: vec![ValueId::new("v")],
            kind: OperationKind::ConstI64(7),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        we.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("v")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        worker.add_block(we);
        module.add_function(worker).unwrap();

        let mut main = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut me = Block::new(Some("entry".into()), loc());
        me.add_op(Operation {
            results: vec![ValueId::new("h")],
            kind: OperationKind::Spawn {
                callee: Symbol::new("worker"),
            },
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        me.add_op(Operation {
            results: vec![ValueId::new("r")],
            kind: OperationKind::Await,
            operands: vec![ValueId::new("h")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        me.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("r")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        main.add_block(me);
        module.add_function(main).unwrap();

        let (result, trace) = run_main_traced(&module).expect("traced spawn+await");
        assert_eq!(result, Some(7));
        // Should have TaskSpawned and TaskAwaited events
        let spawned = trace
            .events()
            .iter()
            .any(|e| matches!(&e.kind, TraceEventKind::TaskSpawned { .. }));
        let awaited = trace
            .events()
            .iter()
            .any(|e| matches!(&e.kind, TraceEventKind::TaskAwaited { .. }));
        assert!(spawned, "trace should contain TaskSpawned event");
        assert!(awaited, "trace should contain TaskAwaited event");
    }

    #[test]
    fn checkpoint_saves_continuation() {
        let mut module = Module::new(Symbol::new("m"));

        let mut main = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("x")],
            kind: OperationKind::ConstI64(99),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
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
            operands: vec![ValueId::new("x")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        main.add_block(entry);
        module.add_function(main).unwrap();

        let (result, trace) = run_main_traced(&module).expect("checkpoint should succeed");
        assert_eq!(result, Some(99));
        let has_checkpoint = trace
            .events()
            .iter()
            .any(|e| matches!(&e.kind, TraceEventKind::Checkpoint { .. }));
        assert!(has_checkpoint, "trace should contain Checkpoint event");
    }

    #[test]
    fn async_runtime_unit() {
        let mut rt = AsyncRuntime::new();
        let id = rt.spawn_task();
        assert_eq!(id, "task_0");
        rt.complete_task(&id, Some(ValueData::Int(10)));
        let result = rt.await_task(&id).unwrap();
        assert!(matches!(result, Some(ValueData::Int(10))));

        // Checkpoint store
        let cont = arc_async::Continuation::new(arc_async::TaskId::new("t1"), "cp1", "i64", 0);
        rt.checkpoint_store.save(&cont).unwrap();
        let loaded = rt.checkpoint_store.load("cp1").unwrap();
        assert_eq!(loaded.label, "cp1");
    }
}
