use arc_ir::{Function, Location, Module, OperationKind, Type, ValueId};
use std::collections::{hash_map::Entry, HashMap, HashSet, VecDeque};

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct VerifyError {
    message: String,
}

impl VerifyError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Clone)]
struct Definition {
    ty: Type,
    origin: DefinitionOrigin,
    location: Location,
    is_resource: bool,
}

#[derive(Clone, Copy)]
enum DefinitionOrigin {
    Param,
    BlockArg { block: usize },
    Op { block: usize },
}

pub fn verify_module(module: &Module) -> Result<(), VerifyError> {
    for func in module.functions.values() {
        verify_function(module, func)?;
    }

    Ok(())
}

fn verify_function(module: &Module, func: &Function) -> Result<(), VerifyError> {
    if func.blocks.is_empty() {
        return Err(VerifyError::new(format!(
            "function {} has no blocks",
            func.name
        )));
    }

    let block_count = func.blocks.len();

    let mut index_params: HashSet<String> = HashSet::new();
    for param in &func.index_params {
        let name = param.name.as_str().to_string();
        if !index_params.insert(name.clone()) {
            return Err(VerifyError::new(format!(
                "duplicate index parameter %{}",
                name
            )));
        }
    }
    if let Some(result_ty) = &func.result {
        validate_type_indices(result_ty, &index_params)?;
    }

    // Collect block labels and ensure uniqueness.
    let mut label_map: HashMap<String, usize> = HashMap::new();
    for (idx, block) in func.blocks.iter().enumerate() {
        if let Some(label) = block.label() {
            let label_str = label.as_str();
            if label_map.insert(label_str.to_string(), idx).is_some() {
                return Err(VerifyError::new(format!(
                    "duplicate block label {}",
                    label_str
                )));
            }
        }
    }

    // Build successor and predecessor lists.
    let mut succs = vec![Vec::new(); block_count];
    let mut preds = vec![Vec::new(); block_count];
    for (idx, block) in func.blocks.iter().enumerate() {
        for op in &block.ops {
            match &op.kind {
                OperationKind::Branch { target } => {
                    let target_idx = lookup_block_index(func, &label_map, target.label.as_str())?;
                    succs[idx].push(target_idx);
                    preds[target_idx].push(idx);
                }
                OperationKind::CondBranch {
                    true_target,
                    false_target,
                } => {
                    let true_idx =
                        lookup_block_index(func, &label_map, true_target.label.as_str())?;
                    let false_idx =
                        lookup_block_index(func, &label_map, false_target.label.as_str())?;
                    succs[idx].push(true_idx);
                    succs[idx].push(false_idx);
                    preds[true_idx].push(idx);
                    preds[false_idx].push(idx);
                }
                _ => {}
            }
        }
    }

    // Determine reachable order from entry block.
    let order = compute_block_order(func, &succs)?;

    // Compute dominator sets.
    let dominators = compute_dominators(func, &preds)?;

    // Record definitions and verify per block.
    let mut definitions: HashMap<String, Definition> = HashMap::new();
    let mut uses: HashMap<String, usize> = HashMap::new();
    for param in &func.params {
        validate_type_indices(&param.ty, &index_params)?;
        insert_definition(
            &mut definitions,
            param.name.as_str(),
            param.ty.clone(),
            DefinitionOrigin::Param,
            param.location,
        )?;
    }

    for &block_idx in &order {
        let block = &func.blocks[block_idx];

        for arg in &block.args {
            validate_type_indices(&arg.ty, &index_params)?;
            insert_definition(
                &mut definitions,
                arg.name.as_str(),
                arg.ty.clone(),
                DefinitionOrigin::BlockArg { block: block_idx },
                arg.location,
            )?;
        }

        for op in &block.ops {
            match &op.kind {
                OperationKind::ConstI64(_) => {
                    if op.result_types.len() != 1 {
                        return Err(VerifyError::new(
                            "const must declare exactly one result type",
                        ));
                    }
                    if op.results.len() != 1 {
                        return Err(VerifyError::new(
                            "const must produce exactly one result value",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let ty = op.result_types[0].clone();
                    let result = &op.results[0];
                    insert_definition(
                        &mut definitions,
                        result.as_str(),
                        ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                }
                OperationKind::Add
                | OperationKind::Sub
                | OperationKind::Mul
                | OperationKind::Div => {
                    if op.operands.len() != 2 {
                        return Err(VerifyError::new("binary arithmetic requires two operands"));
                    }
                    let lhs_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    let rhs_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[1],
                        &mut uses,
                    )?;
                    if lhs_ty != rhs_ty {
                        return Err(VerifyError::new(
                            "binary arithmetic operands must share type",
                        ));
                    }
                    if op.result_types.len() != 1 {
                        return Err(VerifyError::new(
                            "binary arithmetic must declare exactly one result type",
                        ));
                    }
                    if op.results.len() != 1 {
                        return Err(VerifyError::new(
                            "binary arithmetic must produce exactly one result value",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let result_ty = op.result_types[0].clone();
                    if result_ty != lhs_ty {
                        return Err(VerifyError::new(
                            "binary arithmetic result type must match operand type",
                        ));
                    }
                    let result = &op.results[0];
                    insert_definition(
                        &mut definitions,
                        result.as_str(),
                        result_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                }
                OperationKind::ICmp { .. } => {
                    if op.operands.len() != 2 {
                        return Err(VerifyError::new("icmp requires two operands"));
                    }
                    let lhs_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    let rhs_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[1],
                        &mut uses,
                    )?;
                    if lhs_ty != rhs_ty {
                        return Err(VerifyError::new("icmp operands must share type"));
                    }
                    if op.result_types.len() != 1 {
                        return Err(VerifyError::new(
                            "icmp must declare exactly one result type",
                        ));
                    }
                    if op.results.len() != 1 {
                        return Err(VerifyError::new(
                            "icmp must produce exactly one result value",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let result_ty = op.result_types[0].clone();
                    if result_ty != Type::new("i1") {
                        return Err(VerifyError::new(format!(
                            "icmp result must have type i1, found {}",
                            result_ty.as_str()
                        )));
                    }
                    let result = &op.results[0];
                    insert_definition(
                        &mut definitions,
                        result.as_str(),
                        result_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                }
                OperationKind::Branch { target } => {
                    verify_branch_target(
                        func,
                        &definitions,
                        &dominators,
                        block_idx,
                        target,
                        &mut uses,
                    )?;
                }
                OperationKind::CondBranch {
                    true_target,
                    false_target,
                } => {
                    if op.operands.is_empty() {
                        return Err(VerifyError::new("cond_br missing condition operand"));
                    }
                    let cond_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    if cond_ty != Type::new("i1") {
                        return Err(VerifyError::new(format!(
                            "cond_br condition must have type i1, found {}",
                            cond_ty.as_str()
                        )));
                    }
                    verify_branch_target(
                        func,
                        &definitions,
                        &dominators,
                        block_idx,
                        true_target,
                        &mut uses,
                    )?;
                    verify_branch_target(
                        func,
                        &definitions,
                        &dominators,
                        block_idx,
                        false_target,
                        &mut uses,
                    )?;
                }
                OperationKind::Alloc => {
                    if op.operands.len() != 2 {
                        return Err(VerifyError::new(
                            "arc.alloc expects memory and size operands",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let mem_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    if !is_resource_type(&mem_ty) || mem_ty.as_str() != "!arc.mem" {
                        return Err(VerifyError::new(
                            "arc.alloc first operand must be !arc.mem resource",
                        ));
                    }
                    let size_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[1],
                        &mut uses,
                    )?;
                    if size_ty.as_str() != "index" {
                        return Err(VerifyError::new(format!(
                            "arc.alloc size operand must have type index, found {}",
                            size_ty.as_str()
                        )));
                    }
                    if op.results.len() != 2 || op.result_types.len() != 2 {
                        return Err(VerifyError::new(
                            "arc.alloc must produce updated memory and pointer results",
                        ));
                    }
                    let new_mem_ty = op.result_types[0].clone();
                    if new_mem_ty != mem_ty {
                        return Err(VerifyError::new(format!(
                            "arc.alloc must return updated memory of type {}, found {}",
                            mem_ty.as_str(),
                            new_mem_ty.as_str()
                        )));
                    }
                    let ptr_ty = op.result_types[1].clone();
                    if !is_pointer_type(&ptr_ty) {
                        return Err(VerifyError::new(format!(
                            "arc.alloc second result must be pointer type, found {}",
                            ptr_ty.as_str()
                        )));
                    }
                    insert_definition(
                        &mut definitions,
                        op.results[0].as_str(),
                        new_mem_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                    insert_definition(
                        &mut definitions,
                        op.results[1].as_str(),
                        ptr_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                }
                OperationKind::Load => {
                    if op.operands.len() != 2 {
                        return Err(VerifyError::new(
                            "arc.load expects memory and pointer operands",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let mem_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    if mem_ty.as_str() != "!arc.mem" {
                        return Err(VerifyError::new(format!(
                            "arc.load first operand must be !arc.mem, found {}",
                            mem_ty.as_str()
                        )));
                    }
                    let ptr_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[1],
                        &mut uses,
                    )?;
                    if !is_pointer_type(&ptr_ty) {
                        return Err(VerifyError::new(format!(
                            "arc.load pointer operand must be pointer type, found {}",
                            ptr_ty.as_str()
                        )));
                    }
                    if op.results.len() != 2 || op.result_types.len() != 2 {
                        return Err(VerifyError::new(
                            "arc.load must return updated memory and loaded value",
                        ));
                    }
                    let new_mem_ty = op.result_types[0].clone();
                    if new_mem_ty != mem_ty {
                        return Err(VerifyError::new(format!(
                            "arc.load must return memory of type {}, found {}",
                            mem_ty.as_str(),
                            new_mem_ty.as_str()
                        )));
                    }
                    insert_definition(
                        &mut definitions,
                        op.results[0].as_str(),
                        new_mem_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                    let value_ty = op.result_types[1].clone();
                    insert_definition(
                        &mut definitions,
                        op.results[1].as_str(),
                        value_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                }
                OperationKind::LoadElem => {
                    if op.operands.len() < 2 {
                        return Err(VerifyError::new(
                            "arc.load_elem expects slice and index operands",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let slice_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    if !slice_ty.as_str().starts_with("!arc.slice<") {
                        return Err(VerifyError::new(format!(
                            "arc.load_elem first operand must be slice type, found {}",
                            slice_ty.as_str()
                        )));
                    }
                    let index_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[1],
                        &mut uses,
                    )?;
                    if index_ty.as_str() != "index" {
                        return Err(VerifyError::new(format!(
                            "arc.load_elem index must have type index, found {}",
                            index_ty.as_str()
                        )));
                    }
                    for proof_operand in &op.operands[2..] {
                        let proof_ty = check_operand(
                            &definitions,
                            &dominators,
                            block_idx,
                            proof_operand,
                            &mut uses,
                        )?;
                        if !is_proof_type(&proof_ty) {
                            return Err(VerifyError::new(format!(
                                "arc.load_elem requires proof operands, found {}",
                                proof_ty.as_str()
                            )));
                        }
                    }
                    if op.results.len() != 1 || op.result_types.len() != 1 {
                        return Err(VerifyError::new(
                            "arc.load_elem must produce exactly one result value and type",
                        ));
                    }
                    insert_definition(
                        &mut definitions,
                        op.results[0].as_str(),
                        op.result_types[0].clone(),
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                }
                OperationKind::Store => {
                    if op.operands.len() != 3 {
                        return Err(VerifyError::new(
                            "arc.store expects memory, pointer, and value operands",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let mem_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    if mem_ty.as_str() != "!arc.mem" {
                        return Err(VerifyError::new(format!(
                            "arc.store first operand must be !arc.mem, found {}",
                            mem_ty.as_str()
                        )));
                    }
                    let ptr_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[1],
                        &mut uses,
                    )?;
                    if !is_pointer_type(&ptr_ty) {
                        return Err(VerifyError::new(format!(
                            "arc.store pointer operand must be pointer type, found {}",
                            ptr_ty.as_str()
                        )));
                    }
                    let value_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[2],
                        &mut uses,
                    )?;
                    if op.results.len() != 1 || op.result_types.len() != 1 {
                        return Err(VerifyError::new(
                            "arc.store must return updated memory resource",
                        ));
                    }
                    let new_mem_ty = op.result_types[0].clone();
                    if new_mem_ty != mem_ty {
                        return Err(VerifyError::new(format!(
                            "arc.store must return memory of type {}, found {}",
                            mem_ty.as_str(),
                            new_mem_ty.as_str()
                        )));
                    }
                    insert_definition(
                        &mut definitions,
                        op.results[0].as_str(),
                        new_mem_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                    // For now we simply ensure the value was type-checked; more precise
                    // element-type validation will come with richer pointer typing.
                    let _ = value_ty;
                }
                OperationKind::Assume => {
                    if op.operands.len() != 1 {
                        return Err(VerifyError::new(
                            "arc.assume expects exactly one condition operand",
                        ));
                    }
                    let cond_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    if cond_ty != Type::new("i1") {
                        return Err(VerifyError::new(format!(
                            "arc.assume condition must have type i1, found {}",
                            cond_ty.as_str()
                        )));
                    }
                    if op.results.len() != 1 || op.result_types.len() != 1 {
                        return Err(VerifyError::new(
                            "arc.assume must produce exactly one proof result",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let proof_ty = op.result_types[0].clone();
                    if !is_proof_type(&proof_ty) {
                        return Err(VerifyError::new(format!(
                            "arc.assume result must be proof type, found {}",
                            proof_ty.as_str()
                        )));
                    }
                    insert_definition(
                        &mut definitions,
                        op.results[0].as_str(),
                        proof_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                }
                OperationKind::Prove => {
                    if op.operands.len() != 1 {
                        return Err(VerifyError::new("arc.prove expects exactly one operand"));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let cond_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    if cond_ty != Type::new("i1") {
                        return Err(VerifyError::new(format!(
                            "arc.prove operand must have type i1, found {}",
                            cond_ty.as_str()
                        )));
                    }
                    if op.results.len() != 1 || op.result_types.len() != 1 {
                        return Err(VerifyError::new(
                            "arc.prove must produce exactly one proof result",
                        ));
                    }
                    let proof_ty = op.result_types[0].clone();
                    if !is_proof_type(&proof_ty) {
                        return Err(VerifyError::new(format!(
                            "arc.prove result must be proof type, found {}",
                            proof_ty.as_str()
                        )));
                    }
                    insert_definition(
                        &mut definitions,
                        op.results[0].as_str(),
                        proof_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                }
                OperationKind::Refine => {
                    if op.operands.len() != 2 {
                        return Err(VerifyError::new(
                            "arc.refine expects value and proof operands",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let value_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    let proof_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[1],
                        &mut uses,
                    )?;
                    if !is_proof_type(&proof_ty) {
                        return Err(VerifyError::new(format!(
                            "arc.refine second operand must be proof type, found {}",
                            proof_ty.as_str()
                        )));
                    }
                    if op.results.len() != 1 || op.result_types.len() != 1 {
                        return Err(VerifyError::new(
                            "arc.refine must produce exactly one result",
                        ));
                    }
                    let refined_ty = op.result_types[0].clone();
                    let value_ctor = type_constructor(&value_ty);
                    let refined_ctor = type_constructor(&refined_ty);
                    if value_ctor != refined_ctor {
                        return Err(VerifyError::new(format!(
                            "arc.refine result type must share constructor with value type (value {}, result {})",
                            value_ty.as_str(),
                            refined_ty.as_str()
                        )));
                    }
                    insert_definition(
                        &mut definitions,
                        op.results[0].as_str(),
                        refined_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                }
                OperationKind::Assert => {
                    if !op.results.is_empty() {
                        return Err(VerifyError::new("arc.assert does not produce results"));
                    }
                    if op.operands.len() != 1 {
                        return Err(VerifyError::new(
                            "arc.assert expects exactly one condition operand",
                        ));
                    }
                    let cond_ty = check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    if cond_ty != Type::new("i1") {
                        return Err(VerifyError::new(format!(
                            "arc.assert condition must have type i1, found {}",
                            cond_ty.as_str()
                        )));
                    }
                    if op.result_types.len() > 1 {
                        return Err(VerifyError::new(
                            "arc.assert may have at most one type annotation",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    if let Some(annotation) = op.result_types.first() {
                        if annotation != &cond_ty {
                            return Err(VerifyError::new(format!(
                                "arc.assert annotation must match operand type (expected {}, found {})",
                                cond_ty.as_str(),
                                annotation.as_str()
                            )));
                        }
                    }
                }
                OperationKind::Call { callee } => {
                    // Verify the callee exists
                    let callee_func = module.functions.get(callee).ok_or_else(|| {
                        VerifyError::new(format!(
                            "arc.call references undefined function {}",
                            callee
                        ))
                    })?;
                    // Verify argument count matches
                    if op.operands.len() != callee_func.params.len() {
                        return Err(VerifyError::new(format!(
                            "arc.call {} expects {} arguments but got {}",
                            callee,
                            callee_func.params.len(),
                            op.operands.len()
                        )));
                    }
                    // Verify argument types
                    for (i, operand) in op.operands.iter().enumerate() {
                        let operand_ty = check_operand(
                            &definitions,
                            &dominators,
                            block_idx,
                            operand,
                            &mut uses,
                        )?;
                        let param_ty = &callee_func.params[i].ty;
                        if &operand_ty != param_ty {
                            return Err(VerifyError::new(format!(
                                "arc.call {} argument {} type mismatch: expected {}, found {}",
                                callee,
                                i,
                                param_ty.as_str(),
                                operand_ty.as_str()
                            )));
                        }
                    }
                    // Register results
                    if let Some(result_ty) = &callee_func.result {
                        if op.results.len() != 1 {
                            return Err(VerifyError::new(format!(
                                "arc.call {} returns a value, expected one result",
                                callee
                            )));
                        }
                        insert_definition(
                            &mut definitions,
                            op.results[0].as_str(),
                            result_ty.clone(),
                            DefinitionOrigin::Op { block: block_idx },
                            op.location,
                        )?;
                    } else if !op.results.is_empty() {
                        return Err(VerifyError::new(format!(
                            "arc.call {} returns void but has result bindings",
                            callee
                        )));
                    }
                }
                OperationKind::RequireApproval => {
                    // require_approval takes two operands (action, resource) and produces one auth token
                    if op.operands.len() != 2 {
                        return Err(VerifyError::new(
                            "arc.require_approval expects exactly two operands",
                        ));
                    }
                    for operand in &op.operands {
                        check_operand(&definitions, &dominators, block_idx, operand, &mut uses)?;
                    }
                    if op.results.len() != 1 {
                        return Err(VerifyError::new(
                            "arc.require_approval must produce exactly one result",
                        ));
                    }
                    if op.result_types.len() != 1 {
                        return Err(VerifyError::new(
                            "arc.require_approval must declare exactly one result type",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let result_ty = op.result_types[0].clone();
                    if !result_ty.as_str().starts_with("!arc.auth") {
                        return Err(VerifyError::new(format!(
                            "arc.require_approval result type must be !arc.auth<...>, found {}",
                            result_ty.as_str()
                        )));
                    }
                    if let Some(capability_name) = auth_capability_name(&result_ty) {
                        if !module
                            .capabilities
                            .keys()
                            .any(|name| name.as_str() == capability_name)
                        {
                            return Err(VerifyError::new(format!(
                                "arc.require_approval references undefined capability {}",
                                capability_name
                            )));
                        }
                    }
                    insert_definition(
                        &mut definitions,
                        op.results[0].as_str(),
                        result_ty,
                        DefinitionOrigin::Op { block: block_idx },
                        op.location,
                    )?;
                }
                OperationKind::Invoke { capability } => {
                    // Verify capability exists in module
                    let cap = module.capabilities.get(capability).ok_or_else(|| {
                        VerifyError::new(format!(
                            "arc.invoke references undefined capability {}",
                            capability
                        ))
                    })?;
                    // Verify operand count matches capability inputs
                    if op.operands.len() != cap.inputs.len() {
                        return Err(VerifyError::new(format!(
                            "arc.invoke {} expects {} arguments but got {}",
                            capability,
                            cap.inputs.len(),
                            op.operands.len()
                        )));
                    }
                    // Verify operand types
                    for (i, operand) in op.operands.iter().enumerate() {
                        let operand_ty = check_operand(
                            &definitions,
                            &dominators,
                            block_idx,
                            operand,
                            &mut uses,
                        )?;
                        let param_ty = &cap.inputs[i].ty;
                        if &operand_ty != param_ty {
                            return Err(VerifyError::new(format!(
                                "arc.invoke {} argument {} type mismatch: expected {}, found {}",
                                capability,
                                i,
                                param_ty.as_str(),
                                operand_ty.as_str()
                            )));
                        }
                    }
                    // Register results based on capability outputs
                    if op.results.len() != cap.outputs.len() {
                        return Err(VerifyError::new(format!(
                            "arc.invoke {} declares {} outputs but has {} result bindings",
                            capability,
                            cap.outputs.len(),
                            op.results.len()
                        )));
                    }
                    if !op.result_types.is_empty() {
                        if op.result_types.len() != cap.outputs.len() {
                            return Err(VerifyError::new(format!(
                                "arc.invoke {} declares {} output types but capability has {} outputs",
                                capability,
                                op.result_types.len(),
                                cap.outputs.len()
                            )));
                        }
                        for (i, (actual, expected)) in
                            op.result_types.iter().zip(&cap.outputs).enumerate()
                        {
                            if actual != &expected.ty {
                                return Err(VerifyError::new(format!(
                                    "arc.invoke {} output {} type mismatch: expected {}, found {}",
                                    capability,
                                    i,
                                    expected.ty.as_str(),
                                    actual.as_str()
                                )));
                            }
                        }
                    }
                    if !has_available_authority(
                        &definitions,
                        &dominators,
                        block_idx,
                        capability.as_str(),
                    ) {
                        return Err(VerifyError::new(format!(
                            "arc.invoke {} requires available authority token !arc.auth<{}>",
                            capability,
                            capability.as_str()
                        )));
                    }
                    for (i, result_id) in op.results.iter().enumerate() {
                        let result_ty = cap.outputs[i].ty.clone();
                        insert_definition(
                            &mut definitions,
                            result_id.as_str(),
                            result_ty,
                            DefinitionOrigin::Op { block: block_idx },
                            op.location,
                        )?;
                    }
                }
                OperationKind::Return => {
                    if !op.result_types.is_empty() {
                        validate_type_list(&op.result_types, &index_params)?;
                    }
                    if !op.results.is_empty() {
                        return Err(VerifyError::new("return must not declare result values"));
                    }
                    if op.result_types.len() > 1 {
                        return Err(VerifyError::new(
                            "return may have at most one explicit type annotation",
                        ));
                    }
                    let expected = func.result.as_ref();
                    if let Some(annotation) = op.result_types.first() {
                        match expected {
                            Some(expected_ty) => {
                                if annotation != expected_ty {
                                    return Err(VerifyError::new(format!(
                                        "return annotation mismatch: expected {}, found {}",
                                        expected_ty.as_str(),
                                        annotation.as_str()
                                    )));
                                }
                            }
                            None => {
                                return Err(VerifyError::new(
                                    "void function return cannot have type annotation",
                                ));
                            }
                        }
                    }
                    match (expected, op.operands.first()) {
                        (Some(expected_ty), Some(value)) => {
                            let operand_ty = check_operand(
                                &definitions,
                                &dominators,
                                block_idx,
                                value,
                                &mut uses,
                            )?;
                            if &operand_ty != expected_ty {
                                return Err(VerifyError::new(format!(
                                    "return type mismatch: expected {}, found {}",
                                    expected_ty.as_str(),
                                    operand_ty.as_str(),
                                )));
                            }
                        }
                        (None, Some(_)) => {
                            return Err(VerifyError::new(
                                "return with value in function returning void",
                            ));
                        }
                        (Some(_), None) => {
                            return Err(VerifyError::new(
                                "missing return value in function with result",
                            ));
                        }
                        (None, None) => {}
                    }
                }
                OperationKind::If => {
                    // Verify condition operand exists
                    if op.operands.is_empty() {
                        return Err(VerifyError::new("arc.if requires a condition operand"));
                    }
                    check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    // Verify two regions (then, else)
                    if op.regions.len() != 2 {
                        return Err(VerifyError::new(
                            "arc.if requires exactly two regions (then, else)",
                        ));
                    }
                    // Verify region bodies
                    for region in &op.regions {
                        verify_region_body(region, &definitions)?;
                    }
                    // Register results — result_types may be empty for structured ops
                    for (i, result) in op.results.iter().enumerate() {
                        let ty = op
                            .result_types
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| Type::new("i64"));
                        insert_definition(
                            &mut definitions,
                            result.as_str(),
                            ty,
                            DefinitionOrigin::Op { block: block_idx },
                            op.location,
                        )?;
                    }
                }
                OperationKind::Loop { .. } => {
                    // Verify body region exists
                    if op.regions.is_empty() {
                        return Err(VerifyError::new("arc.loop requires a body region"));
                    }
                    // Verify region body
                    for region in &op.regions {
                        verify_region_body(region, &definitions)?;
                    }
                    // Register results — result_types may be empty for structured ops
                    for (i, result) in op.results.iter().enumerate() {
                        let ty = op
                            .result_types
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| Type::new("i64"));
                        insert_definition(
                            &mut definitions,
                            result.as_str(),
                            ty,
                            DefinitionOrigin::Op { block: block_idx },
                            op.location,
                        )?;
                    }
                }
                OperationKind::Yield => {
                    // Yield terminates a region body — operands are the yielded values
                    for operand in &op.operands {
                        check_operand(&definitions, &dominators, block_idx, operand, &mut uses)?;
                    }
                }
                OperationKind::Spawn { callee } => {
                    // Verify the callee exists in module
                    if !module.functions.contains_key(callee) {
                        return Err(VerifyError::new(format!(
                            "arc.spawn references undefined function @{}",
                            callee
                        )));
                    }
                    for operand in &op.operands {
                        check_operand(&definitions, &dominators, block_idx, operand, &mut uses)?;
                    }
                    for (i, result) in op.results.iter().enumerate() {
                        let ty = op
                            .result_types
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| Type::new("i64"));
                        insert_definition(
                            &mut definitions,
                            result.as_str(),
                            ty,
                            DefinitionOrigin::Op { block: block_idx },
                            op.location,
                        )?;
                    }
                }
                OperationKind::Await => {
                    if op.operands.is_empty() {
                        return Err(VerifyError::new("arc.await requires a task handle operand"));
                    }
                    check_operand(
                        &definitions,
                        &dominators,
                        block_idx,
                        &op.operands[0],
                        &mut uses,
                    )?;
                    for (i, result) in op.results.iter().enumerate() {
                        let ty = op
                            .result_types
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| Type::new("i64"));
                        insert_definition(
                            &mut definitions,
                            result.as_str(),
                            ty,
                            DefinitionOrigin::Op { block: block_idx },
                            op.location,
                        )?;
                    }
                }
                OperationKind::Checkpoint { .. } => {
                    for operand in &op.operands {
                        check_operand(&definitions, &dominators, block_idx, operand, &mut uses)?;
                    }
                    for (i, result) in op.results.iter().enumerate() {
                        let ty = op
                            .result_types
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| Type::new("i64"));
                        insert_definition(
                            &mut definitions,
                            result.as_str(),
                            ty,
                            DefinitionOrigin::Op { block: block_idx },
                            op.location,
                        )?;
                    }
                }
                OperationKind::Unknown(name) => {
                    return Err(VerifyError::new(format!("unsupported operation {}", name)));
                }
            }
        }

        let terminator = block
            .terminator()
            .ok_or_else(|| VerifyError::new("block missing terminator"))?;
        match terminator.kind {
            OperationKind::Return
            | OperationKind::Branch { .. }
            | OperationKind::CondBranch { .. } => {}
            _ => {
                return Err(VerifyError::new(
                    "block terminator must be branch or return",
                ));
            }
        }
    }

    verify_proof_availability(func, &label_map)?;
    check_resource_linearity(&definitions, &uses)
}

fn lookup_block_index(
    func: &Function,
    label_map: &HashMap<String, usize>,
    label: &str,
) -> Result<usize, VerifyError> {
    if let Some(&idx) = label_map.get(label) {
        Ok(idx)
    } else {
        // Fall back to scan in case the block is unlabeled but first block matches.
        func.block_index_by_label(label)
            .ok_or_else(|| VerifyError::new(format!("branch target {} not found", label)))
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
struct ProofFacts {
    booleans: HashSet<ValueId>,
    proofs: HashSet<ValueId>,
}

impl ProofFacts {
    fn insert_boolean(&mut self, value: &ValueId) {
        self.booleans.insert(value.clone());
    }

    fn remove_boolean(&mut self, value: &ValueId) {
        self.booleans.remove(value);
    }

    fn insert_proof(&mut self, value: &ValueId) {
        self.proofs.insert(value.clone());
    }

    fn remove_proof(&mut self, value: &ValueId) {
        self.proofs.remove(value);
    }

    fn retain_intersection(&mut self, other: &ProofFacts) {
        self.booleans.retain(|value| other.booleans.contains(value));
        self.proofs.retain(|value| other.proofs.contains(value));
    }
}

fn verify_proof_availability(
    func: &Function,
    label_map: &HashMap<String, usize>,
) -> Result<(), VerifyError> {
    let block_count = func.blocks.len();
    let mut block_facts_in: Vec<ProofFacts> = vec![ProofFacts::default(); block_count];
    let mut per_pred_facts: Vec<HashMap<usize, ProofFacts>> = vec![HashMap::new(); block_count];
    let mut proof_conditions: HashMap<ValueId, ValueId> = HashMap::new();
    let mut worklist = VecDeque::new();
    worklist.push_back(0);

    while let Some(block_idx) = worklist.pop_front() {
        let block = &func.blocks[block_idx];
        let mut facts = block_facts_in[block_idx].clone();

        for op in &block.ops {
            match &op.kind {
                OperationKind::Assume => {
                    if let Some(cond) = op.operands.first() {
                        facts.insert_boolean(cond);
                        if let Some(result) = op.results.first() {
                            facts.insert_proof(result);
                            proof_conditions.insert(result.clone(), cond.clone());
                        }
                    }
                }
                OperationKind::Assert => {
                    if let Some(cond) = op.operands.first() {
                        facts.insert_boolean(cond);
                    }
                }
                OperationKind::Prove => {
                    let cond = op
                        .operands
                        .first()
                        .expect("arc.prove already validated to have operand")
                        .clone();
                    if !facts.booleans.contains(&cond) {
                        return Err(VerifyError::new(format!(
                            "arc.prove operand {} is not established as true on all incoming paths",
                            cond
                        )));
                    }
                    if let Some(result) = op.results.first() {
                        facts.insert_proof(result);
                        proof_conditions.insert(result.clone(), cond);
                    }
                }
                OperationKind::LoadElem => {
                    let proof_operands = &op.operands[2..];
                    for proof_operand in proof_operands {
                        if !facts.proofs.contains(proof_operand) {
                            return Err(VerifyError::new(format!(
                                "arc.load_elem requires proof {} to hold on all incoming paths",
                                proof_operand
                            )));
                        }
                    }
                    if proof_operands.is_empty() && facts.proofs.is_empty() {
                        return Err(VerifyError::new(
                            "arc.load_elem requires a proof to be established on all incoming paths",
                        ));
                    }
                }
                OperationKind::Branch { target } => {
                    let succ_idx = lookup_block_index(func, label_map, target.label.as_str())?;
                    propagate_proof_facts(
                        block_idx,
                        succ_idx,
                        facts.clone(),
                        &mut per_pred_facts,
                        &mut block_facts_in,
                        &mut worklist,
                    );
                }
                OperationKind::CondBranch {
                    true_target,
                    false_target,
                } => {
                    let cond = op
                        .operands
                        .first()
                        .expect("cond_br must have a condition operand");
                    let mut true_facts = facts.clone();
                    true_facts.insert_boolean(cond);
                    let mut false_facts = facts.clone();
                    prune_proofs_for_condition(&mut false_facts, cond, &proof_conditions);
                    false_facts.remove_boolean(cond);
                    let true_idx = lookup_block_index(func, label_map, true_target.label.as_str())?;
                    let false_idx =
                        lookup_block_index(func, label_map, false_target.label.as_str())?;
                    propagate_proof_facts(
                        block_idx,
                        true_idx,
                        true_facts,
                        &mut per_pred_facts,
                        &mut block_facts_in,
                        &mut worklist,
                    );
                    propagate_proof_facts(
                        block_idx,
                        false_idx,
                        false_facts,
                        &mut per_pred_facts,
                        &mut block_facts_in,
                        &mut worklist,
                    );
                    break;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn prune_proofs_for_condition(
    facts: &mut ProofFacts,
    cond: &ValueId,
    proof_conditions: &HashMap<ValueId, ValueId>,
) {
    let mut to_remove = Vec::new();
    for proof in &facts.proofs {
        if let Some(source_cond) = proof_conditions.get(proof) {
            if source_cond == cond {
                to_remove.push(proof.clone());
            }
        }
    }
    for proof in to_remove {
        facts.remove_proof(&proof);
    }
}

fn propagate_proof_facts(
    from_block: usize,
    succ_block: usize,
    proposal: ProofFacts,
    per_pred_facts: &mut [HashMap<usize, ProofFacts>],
    block_facts_in: &mut [ProofFacts],
    worklist: &mut VecDeque<usize>,
) {
    let mut needs_visit = false;
    match per_pred_facts[succ_block].entry(from_block) {
        Entry::Vacant(slot) => {
            slot.insert(proposal);
            needs_visit = true;
        }
        Entry::Occupied(mut slot) => {
            if slot.get() != &proposal {
                slot.insert(proposal);
                needs_visit = true;
            }
        }
    }

    let mut intersection: Option<ProofFacts> = None;
    for env in per_pred_facts[succ_block].values() {
        match &mut intersection {
            None => {
                intersection = Some(env.clone());
            }
            Some(acc) => {
                acc.retain_intersection(env);
            }
        }
    }
    let new_facts = intersection.unwrap_or_default();
    if block_facts_in[succ_block] != new_facts {
        block_facts_in[succ_block] = new_facts;
        needs_visit = true;
    }
    if needs_visit {
        worklist.push_back(succ_block);
    }
}

fn compute_block_order(func: &Function, succs: &[Vec<usize>]) -> Result<Vec<usize>, VerifyError> {
    let mut visited = vec![false; succs.len()];
    let mut queue = VecDeque::new();
    let mut order = Vec::new();

    visited[0] = true;
    queue.push_back(0);

    while let Some(block_idx) = queue.pop_front() {
        order.push(block_idx);
        for &succ in &succs[block_idx] {
            if !visited[succ] {
                visited[succ] = true;
                queue.push_back(succ);
            }
        }
    }

    for (idx, block) in func.blocks.iter().enumerate() {
        if !visited[idx] {
            return Err(VerifyError::new(format!(
                "block {} is unreachable",
                block
                    .label()
                    .map(|label| label.as_str().to_string())
                    .unwrap_or_else(|| format!("#{}", idx))
            )));
        }
    }

    Ok(order)
}

fn compute_dominators(
    func: &Function,
    preds: &[Vec<usize>],
) -> Result<Vec<Vec<bool>>, VerifyError> {
    let n = func.blocks.len();
    let mut dom = vec![vec![true; n]; n];
    dom[0] = vec![false; n];
    dom[0][0] = true;

    let mut changed = true;
    while changed {
        changed = false;
        for b in 1..n {
            if preds[b].is_empty() {
                return Err(VerifyError::new(format!(
                    "block {} has no predecessors",
                    func.blocks[b]
                        .label()
                        .map(|label| label.as_str().to_string())
                        .unwrap_or_else(|| format!("#{}", b))
                )));
            }
            let mut new_dom = vec![true; n];
            for j in 0..n {
                for &pred in &preds[b] {
                    if !dom[pred][j] {
                        new_dom[j] = false;
                        break;
                    }
                }
            }
            new_dom[b] = true;
            if new_dom != dom[b] {
                dom[b] = new_dom;
                changed = true;
            }
        }
    }

    Ok(dom)
}

fn insert_definition(
    definitions: &mut HashMap<String, Definition>,
    name: &str,
    ty: Type,
    origin: DefinitionOrigin,
    location: Location,
) -> Result<(), VerifyError> {
    if definitions.contains_key(name) {
        return Err(VerifyError::new(format!(
            "value {} redefined in the same function",
            name
        )));
    }
    let is_resource = is_resource_type(&ty);
    definitions.insert(
        name.to_string(),
        Definition {
            ty,
            origin,
            location,
            is_resource,
        },
    );
    Ok(())
}

fn check_operand(
    definitions: &HashMap<String, Definition>,
    dominators: &[Vec<bool>],
    block_idx: usize,
    value: &ValueId,
    uses: &mut HashMap<String, usize>,
) -> Result<Type, VerifyError> {
    let def = definitions
        .get(value.as_str())
        .ok_or_else(|| VerifyError::new(format!("use of undefined value {}", value)))?;
    match def.origin {
        DefinitionOrigin::Param => {}
        DefinitionOrigin::BlockArg { block } | DefinitionOrigin::Op { block } => {
            if !dominators[block_idx][block] {
                return Err(VerifyError::new(format!(
                    "value {} does not dominate its use",
                    value
                )));
            }
        }
    }
    record_use(uses, value);
    Ok(def.ty.clone())
}

fn has_available_authority(
    definitions: &HashMap<String, Definition>,
    dominators: &[Vec<bool>],
    block_idx: usize,
    capability: &str,
) -> bool {
    definitions.values().any(|def| {
        auth_capability_name(&def.ty) == Some(capability)
            && definition_available(def, dominators, block_idx)
    })
}

fn auth_capability_name(ty: &Type) -> Option<&str> {
    ty.as_str()
        .strip_prefix("!arc.auth<")
        .and_then(|inner| inner.strip_suffix('>'))
}

fn definition_available(def: &Definition, dominators: &[Vec<bool>], block_idx: usize) -> bool {
    match def.origin {
        DefinitionOrigin::Param => true,
        DefinitionOrigin::BlockArg { block } | DefinitionOrigin::Op { block } => {
            dominators[block_idx][block]
        }
    }
}

fn verify_branch_target(
    func: &Function,
    definitions: &HashMap<String, Definition>,
    dominators: &[Vec<bool>],
    current_block: usize,
    target: &arc_ir::BlockTarget,
    uses: &mut HashMap<String, usize>,
) -> Result<(), VerifyError> {
    let dest_idx = func
        .block_index_by_label(target.label.as_str())
        .ok_or_else(|| VerifyError::new(format!("branch target {} not found", target.label)))?;
    let dest_block = &func.blocks[dest_idx];
    if dest_block.args.len() != target.arguments.len() {
        return Err(VerifyError::new(format!(
            "branch to {} expected {} arguments but found {}",
            target.label.as_str(),
            dest_block.args.len(),
            target.arguments.len()
        )));
    }
    for (value_id, arg) in target.arguments.iter().zip(&dest_block.args) {
        let ty = check_operand(definitions, dominators, current_block, value_id, uses)?;
        if ty != arg.ty {
            return Err(VerifyError::new(format!(
                "branch argument {} type mismatch (expected {}, found {})",
                value_id,
                arg.ty.as_str(),
                ty.as_str()
            )));
        }
    }
    Ok(())
}

fn check_resource_linearity(
    definitions: &HashMap<String, Definition>,
    uses: &HashMap<String, usize>,
) -> Result<(), VerifyError> {
    for (name, def) in definitions {
        if !def.is_resource {
            continue;
        }
        let use_count = uses.get(name).cloned().unwrap_or(0);
        if use_count == 0 {
            return Err(VerifyError::new(format!(
                "resource value %{} (defined at offset {}) is never used",
                name, def.location.offset
            )));
        }
        if use_count > 1 {
            return Err(VerifyError::new(format!(
                "resource value %{} (defined at offset {}) used more than once",
                name, def.location.offset
            )));
        }
    }
    Ok(())
}

fn record_use(uses: &mut HashMap<String, usize>, value: &ValueId) {
    *uses.entry(value.as_str().to_string()).or_default() += 1;
}

/// Verify operations inside a structured region body.
/// Region bodies inherit definitions from the parent scope but define their own local values.
fn verify_region_body(
    region: &arc_ir::Region,
    parent_defs: &HashMap<String, Definition>,
) -> Result<(), VerifyError> {
    // Create a local scope that inherits from the parent
    let mut local_defs = parent_defs.clone();

    for block in &region.blocks {
        // Register block arguments
        for arg in &block.args {
            // Allow shadowing of parent definitions within region scope
            local_defs.insert(
                arg.name.as_str().to_string(),
                Definition {
                    ty: arg.ty.clone(),
                    origin: DefinitionOrigin::Param,
                    location: arg.location,
                    is_resource: is_resource_type(&arg.ty),
                },
            );
        }

        for op in &block.ops {
            // Check operands are defined (either local or inherited from parent)
            match &op.kind {
                OperationKind::Yield => {
                    for operand in &op.operands {
                        if !local_defs.contains_key(operand.as_str()) {
                            return Err(VerifyError::new(format!(
                                "use of undefined value {} in region",
                                operand
                            )));
                        }
                    }
                }
                OperationKind::If => {
                    // Nested if: check condition operand
                    if let Some(cond) = op.operands.first() {
                        if !local_defs.contains_key(cond.as_str()) {
                            return Err(VerifyError::new(format!(
                                "use of undefined value {} in region",
                                cond
                            )));
                        }
                    }
                    // Recurse into nested regions
                    for nested_region in &op.regions {
                        verify_region_body(nested_region, &local_defs)?;
                    }
                }
                OperationKind::Loop { .. } => {
                    for nested_region in &op.regions {
                        verify_region_body(nested_region, &local_defs)?;
                    }
                }
                _ => {
                    // For all other ops, verify operands are defined
                    for operand in &op.operands {
                        if !local_defs.contains_key(operand.as_str()) {
                            return Err(VerifyError::new(format!(
                                "use of undefined value {} in region",
                                operand
                            )));
                        }
                    }
                }
            }

            // Register results in local scope
            for (i, result) in op.results.iter().enumerate() {
                let ty = op
                    .result_types
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| Type::new("i64"));
                local_defs.insert(
                    result.as_str().to_string(),
                    Definition {
                        ty,
                        origin: DefinitionOrigin::Op { block: 0 },
                        location: op.location,
                        is_resource: false,
                    },
                );
            }
        }
    }

    Ok(())
}

fn is_resource_type(ty: &Type) -> bool {
    const RESOURCE_PREFIXES: &[&str] = &[
        "!arc.mem",
        "!arc.fs",
        "!arc.net",
        "!arc.db",
        "!arc.world",
        "!arc.clock",
        "!arc.rng",
        "!arc.ui",
        "!arc.gpu",
        "!arc.vault",
    ];
    let repr = ty.as_str();
    RESOURCE_PREFIXES
        .iter()
        .any(|prefix| repr.starts_with(prefix))
}

fn is_pointer_type(ty: &Type) -> bool {
    ty.as_str().starts_with("!arc.ptr<") || ty.as_str() == "!arc.ptr"
}

fn is_proof_type(ty: &Type) -> bool {
    let repr = ty.as_str();
    repr.starts_with("!arc.proof<") || repr == "!arc.proof"
}

fn type_constructor(ty: &Type) -> &str {
    let repr = ty.as_str();
    let bytes = repr.as_bytes();
    for (idx, b) in bytes.iter().enumerate() {
        match b {
            b'<' | b'(' | b'[' => {
                return &repr[..idx];
            }
            _ => {}
        }
    }
    repr
}

fn validate_type_list(types: &[Type], index_params: &HashSet<String>) -> Result<(), VerifyError> {
    for ty in types {
        validate_type_indices(ty, index_params)?;
    }
    Ok(())
}

fn validate_type_indices(ty: &Type, index_params: &HashSet<String>) -> Result<(), VerifyError> {
    for var in extract_index_vars(ty) {
        if !index_params.contains(&var) {
            return Err(VerifyError::new(format!(
                "type {} references undefined index %{}",
                ty.as_str(),
                var
            )));
        }
    }
    Ok(())
}

fn extract_index_vars(ty: &Type) -> Vec<String> {
    let mut vars = Vec::new();
    let repr = ty.as_str();
    let mut bytes = repr.as_bytes();
    let mut offset = 0;
    while !bytes.is_empty() {
        if bytes[0] == b'%' {
            let mut len = 0;
            for b in &bytes[1..] {
                if b.is_ascii_alphanumeric() || *b == b'_' {
                    len += 1;
                } else {
                    break;
                }
            }
            if len > 0 {
                let start = offset + 1;
                let end = start + len;
                vars.push(repr[start..end].to_string());
                let advance = len + 1;
                bytes = &bytes[advance..];
                offset += advance;
                continue;
            }
        }
        bytes = &bytes[1..];
        offset += 1;
    }
    vars
}

// ---------------------------------------------------------------------------
// Extended verification: security, memory, and proof integration
// ---------------------------------------------------------------------------

/// Result of extended verification including security, memory, and proof checks.
#[derive(Debug)]
pub struct ExtendedVerifyResult {
    /// Security violations found (information flow, taint, sandbox).
    pub security_violations: Vec<arc_security::SecurityViolation>,
    /// Memory safety violations found (use-after-free, bounds, uninit reads).
    pub memory_violations: Vec<arc_memory::MemoryViolation>,
    /// Proof obligations that could not be discharged.
    pub unproved_obligations: Vec<String>,
    /// Security audit summary.
    pub audit: arc_security::SecurityAudit,
}

impl ExtendedVerifyResult {
    /// Returns true if no violations or unproved obligations were found.
    pub fn is_clean(&self) -> bool {
        self.security_violations.is_empty()
            && self.memory_violations.is_empty()
            && self.unproved_obligations.is_empty()
    }

    /// Total number of issues found.
    pub fn issue_count(&self) -> usize {
        self.security_violations.len()
            + self.memory_violations.len()
            + self.unproved_obligations.len()
    }
}

/// Run base verification followed by security, memory, and proof analysis.
///
/// This first calls `verify_module` (type checking, dominance, etc.), then runs
/// the extended analyses. If base verification fails, returns Err immediately.
pub fn verify_module_extended(
    module: &Module,
    security_ctx: &arc_security::SecurityContext,
    sandbox: Option<&arc_security::SandboxPolicy>,
) -> Result<ExtendedVerifyResult, VerifyError> {
    // Base verification must pass first
    verify_module(module)?;

    // Security analysis
    let security_violations = arc_security::check_information_flow(module, security_ctx, sandbox);
    let audit = arc_security::audit_module(module, security_ctx, sandbox);

    // Memory safety analysis
    let memory_violations = arc_memory::verify_memory_safety(module);

    // Proof obligation analysis: check assert/prove ops
    let unproved_obligations = check_proof_obligations(module);

    Ok(ExtendedVerifyResult {
        security_violations,
        memory_violations,
        unproved_obligations,
        audit,
    })
}

/// Check proof obligations from assert/prove operations using the proof kernel.
fn check_proof_obligations(module: &Module) -> Vec<String> {
    let solver = arc_proof::LinearArithmeticSolver;
    let mut unproved = Vec::new();

    for func in module.functions.values() {
        let mut proof_ctx = arc_proof::ProofContext::new();

        // Collect known bounds from function parameters
        for param in &func.params {
            if param.ty.as_str() == "index" || param.ty.as_str() == "i64" {
                // Parameters are unbounded by default; specific bounds would come
                // from assume ops or refinement types
            }
        }

        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OperationKind::Assume => {
                        // Assume adds a fact to the proof context
                        if let Some(cond) = op.operands.first() {
                            proof_ctx.add_fact(arc_proof::Expr::var(cond.as_str()));
                        }
                    }
                    OperationKind::Assert => {
                        // Assert creates an obligation that the condition is true
                        if let Some(cond) = op.operands.first() {
                            let obligation = arc_proof::ProofObligation {
                                kind: arc_proof::ObligationKind::Predicate {
                                    expr: arc_proof::Expr::var(cond.as_str()),
                                },
                                description: format!("assert in {}: %{}", func.name, cond.as_str()),
                                context: proof_ctx.clone(),
                            };
                            let result = arc_proof::discharge_with_solver(&obligation, &solver);
                            if !result.is_proved() {
                                unproved.push(obligation.description);
                            }
                        }
                    }
                    OperationKind::Prove => {
                        // Prove creates a proof obligation that a predicate holds
                        if let Some(cond) = op.operands.first() {
                            let obligation = arc_proof::ProofObligation {
                                kind: arc_proof::ObligationKind::Predicate {
                                    expr: arc_proof::Expr::var(cond.as_str()),
                                },
                                description: format!("prove in {}: %{}", func.name, cond.as_str()),
                                context: proof_ctx.clone(),
                            };
                            let result = arc_proof::discharge_with_solver(&obligation, &solver);
                            if !result.is_proved() {
                                unproved.push(obligation.description);
                            }
                        }
                    }
                    OperationKind::LoadElem => {
                        // Bounds check: if there's a proof operand, verify it
                        if op.operands.len() >= 3 {
                            // Has index proof — check bounds
                            let _base = &op.operands[0];
                            let index = &op.operands[1];
                            let obligation = arc_proof::ProofObligation {
                                kind: arc_proof::ObligationKind::BoundsCheck {
                                    index: arc_proof::Expr::var(index.as_str()),
                                    bound: arc_proof::Expr::var("__array_len"),
                                },
                                description: format!(
                                    "bounds check in {}: %{}",
                                    func.name,
                                    index.as_str()
                                ),
                                context: proof_ctx.clone(),
                            };
                            let result = arc_proof::discharge_with_solver(&obligation, &solver);
                            if !result.is_proved() {
                                unproved.push(obligation.description);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    unproved
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_ir::{
        Argument, Block, BlockTarget, IcmpPredicate, IndexParam, Location, Module, Operation,
        OperationKind, Symbol, Type, ValueId,
    };

    fn loc() -> Location {
        Location::new(0, 0)
    }

    fn minimal_function() -> Function {
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut block = Block::new(Some("entry".into()), loc());
        block.add_op(Operation {
            results: vec![ValueId::new("one")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        block.add_op(Operation {
            results: vec![ValueId::new("sum")],
            kind: OperationKind::Add,
            operands: vec![ValueId::new("one"), ValueId::new("one")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("sum")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(block);
        func
    }

    #[test]
    fn verify_simple_function() {
        let mut module = Module::new(Symbol::new("m"));
        let func = minimal_function();
        module.add_function(func).unwrap();
        verify_module(&module).expect("verification succeeds");
    }

    #[test]
    fn reject_undefined_value() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut block = Block::new(Some("entry".into()), loc());
        block.add_op(Operation {
            results: vec![ValueId::new("sum")],
            kind: OperationKind::Add,
            operands: vec![ValueId::new("missing"), ValueId::new("missing")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(block);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("verification should fail");
        assert!(err.message.contains("missing"), "unexpected error: {}", err);
    }

    #[test]
    fn verify_branch_arguments() {
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
            results: vec![ValueId::new("x")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Branch {
                target: arc_ir::BlockTarget::new("then".into(), vec![ValueId::new("x")]),
            },
            operands: Vec::new(),
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_arg(Argument {
            name: ValueId::new("y"),
            ty: Type::new("i64"),
            location: loc(),
        });
        then_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("y")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        func.add_block(entry);
        func.add_block(then_block);
        module.add_function(func).unwrap();
        verify_module(&module).expect("branch verified");
    }

    #[test]
    fn reject_branch_argument_mismatch() {
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
            results: Vec::new(),
            kind: OperationKind::Branch {
                target: arc_ir::BlockTarget::new("then".into(), vec![]),
            },
            operands: Vec::new(),
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_arg(Argument {
            name: ValueId::new("y"),
            ty: Type::new("i64"),
            location: loc(),
        });
        then_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("y")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        func.add_block(entry);
        func.add_block(then_block);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("verification should fail");
        assert!(
            err.message.contains("expected 1 arguments"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn resource_linear_use() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("mem"),
                ty: Type::new("!arc.mem"),
                location: loc(),
            }],
            Some(Type::new("!arc.mem")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("mem")],
            result_types: vec![Type::new("!arc.mem")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        verify_module(&module).expect("resource used exactly once");
    }

    #[test]
    fn resource_used_twice_is_error() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("mem"),
                ty: Type::new("!arc.mem"),
                location: loc(),
            }],
            Some(Type::new("!arc.mem")),
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
            results: vec![ValueId::new("always_true")],
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
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target: arc_ir::BlockTarget::new("then".into(), vec![ValueId::new("mem")]),
                false_target: arc_ir::BlockTarget::new("else".into(), vec![ValueId::new("mem")]),
            },
            operands: vec![ValueId::new("always_true")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_arg(Argument {
            name: ValueId::new("m1"),
            ty: Type::new("!arc.mem"),
            location: loc(),
        });
        then_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("m1")],
            result_types: vec![Type::new("!arc.mem")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        let mut else_block = Block::new(Some("else".into()), loc());
        else_block.add_arg(Argument {
            name: ValueId::new("m2"),
            ty: Type::new("!arc.mem"),
            location: loc(),
        });
        else_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("m2")],
            result_types: vec![Type::new("!arc.mem")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        func.add_block(then_block);
        func.add_block(else_block);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("resource should not be duplicated");
        assert!(
            err.message.contains("used more than once"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn memory_ops_verify() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![
                Argument {
                    name: ValueId::new("mem"),
                    ty: Type::new("!arc.mem"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("size"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("!arc.mem")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("init")],
            kind: OperationKind::ConstI64(42),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("mem1"), ValueId::new("ptr")],
            kind: OperationKind::Alloc,
            operands: vec![ValueId::new("mem"), ValueId::new("size")],
            result_types: vec![Type::new("!arc.mem"), Type::new("!arc.ptr<i64>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("mem2")],
            kind: OperationKind::Store,
            operands: vec![
                ValueId::new("mem1"),
                ValueId::new("ptr"),
                ValueId::new("init"),
            ],
            result_types: vec![Type::new("!arc.mem")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("mem3"), ValueId::new("loaded")],
            kind: OperationKind::Load,
            operands: vec![ValueId::new("mem2"), ValueId::new("ptr")],
            result_types: vec![Type::new("!arc.mem"), Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("mem3")],
            result_types: vec![Type::new("!arc.mem")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        verify_module(&module).expect("memory ops verify");
    }

    #[test]
    fn alloc_requires_pointer_result_type() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![
                Argument {
                    name: ValueId::new("mem"),
                    ty: Type::new("!arc.mem"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("size"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("!arc.mem")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("mem1"), ValueId::new("not_ptr")],
            kind: OperationKind::Alloc,
            operands: vec![ValueId::new("mem"), ValueId::new("size")],
            result_types: vec![Type::new("!arc.mem"), Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("mem1")],
            result_types: vec![Type::new("!arc.mem")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("alloc should reject non-pointer result");
        assert!(
            err.message.contains("second result must be pointer type"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn store_must_return_memory() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![
                Argument {
                    name: ValueId::new("mem"),
                    ty: Type::new("!arc.mem"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("size"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("!arc.mem")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("val")],
            kind: OperationKind::ConstI64(0),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("mem1"), ValueId::new("ptr")],
            kind: OperationKind::Alloc,
            operands: vec![ValueId::new("mem"), ValueId::new("size")],
            result_types: vec![Type::new("!arc.mem"), Type::new("!arc.ptr<i8>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("mem2")],
            kind: OperationKind::Store,
            operands: vec![
                ValueId::new("mem1"),
                ValueId::new("ptr"),
                ValueId::new("val"),
            ],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("mem2")],
            result_types: vec![Type::new("!arc.mem")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("store must return memory resource");
        assert!(
            err.message.contains("must return memory of type"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn indexed_type_requires_declared_index() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("slice_len"),
            vec![IndexParam {
                name: ValueId::new("n"),
                location: loc(),
            }],
            vec![Argument {
                name: ValueId::new("xs"),
                ty: Type::new("!arc.slice<i32, %n>"),
                location: loc(),
            }],
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("len")],
            kind: OperationKind::ConstI64(0),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
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
            results: vec![ValueId::new("positive")],
            kind: OperationKind::ICmp {
                predicate: IcmpPredicate::Sgt,
            },
            operands: vec![ValueId::new("len"), ValueId::new("zero")],
            result_types: vec![Type::new("i1")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("pf")],
            kind: OperationKind::Assume,
            operands: vec![ValueId::new("positive")],
            result_types: vec![Type::new("!arc.proof<%n > 0>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("len")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        verify_module(&module).expect("declared index should be accepted");
    }

    #[test]
    fn missing_index_parameter_is_error() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("slice_len"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("xs"),
                ty: Type::new("!arc.slice<i32, %n>"),
                location: loc(),
            }],
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("len")],
            kind: OperationKind::ConstI64(0),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("len")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("missing index parameter should fail");
        assert!(
            err.message.contains("undefined index"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn assume_requires_proof_result() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("cond"),
                ty: Type::new("i1"),
                location: loc(),
            }],
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("pf")],
            kind: OperationKind::Assume,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("assume should require proof type");
        assert!(
            err.message.contains("result must be proof type"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn assert_requires_boolean_condition() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("value"),
                ty: Type::new("i64"),
                location: loc(),
            }],
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Assert,
            operands: vec![ValueId::new("value")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("value")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("assert should require boolean condition");
        assert!(
            err.message
                .contains("arc.assert condition must have type i1"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn cond_branch_requires_boolean_condition() {
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
            results: vec![ValueId::new("cond_value")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target: arc_ir::BlockTarget::new("then".into(), vec![]),
                false_target: arc_ir::BlockTarget::new("else".into(), vec![]),
            },
            operands: vec![ValueId::new("cond_value")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("cond_value")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut else_block = Block::new(Some("else".into()), loc());
        else_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("cond_value")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        func.add_block(entry);
        func.add_block(then_block);
        func.add_block(else_block);
        module.add_function(func).unwrap();

        let err = verify_module(&module).expect_err("cond_br should require i1 condition");
        assert!(
            err.message.contains("cond_br condition must have type i1"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn load_elem_with_proof_verifies() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            vec![IndexParam {
                name: ValueId::new("n"),
                location: loc(),
            }],
            vec![
                Argument {
                    name: ValueId::new("xs"),
                    ty: Type::new("!arc.slice<i32, %n>"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("idx"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("i32")),
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
            result_types: vec![Type::new("!arc.proof<%n > 0>")],
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
        verify_module(&module).expect("load_elem with proof should verify");
    }

    #[test]
    fn load_elem_reuses_available_proof_fact() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            vec![IndexParam {
                name: ValueId::new("n"),
                location: loc(),
            }],
            vec![
                Argument {
                    name: ValueId::new("xs"),
                    ty: Type::new("!arc.slice<i32, %n>"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("idx"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("i32")),
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
        verify_module(&module)
            .expect("load_elem should verify when proof fact is available without operand");
    }

    #[test]
    fn load_elem_missing_proof_is_error() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            vec![IndexParam {
                name: ValueId::new("n"),
                location: loc(),
            }],
            vec![
                Argument {
                    name: ValueId::new("xs"),
                    ty: Type::new("!arc.slice<i32, %n>"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("idx"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("i32")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
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
        let err = verify_module(&module).expect_err("load_elem without proof should fail");
        assert!(
            err.message
                .contains("arc.load_elem requires a proof to be established on all incoming paths"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn load_elem_proof_not_available_on_false_branch_is_error() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            vec![IndexParam {
                name: ValueId::new("n"),
                location: loc(),
            }],
            vec![
                Argument {
                    name: ValueId::new("xs"),
                    ty: Type::new("!arc.slice<i32, %n>"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("idx"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("i32")),
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
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target: BlockTarget::new("ok".into(), Vec::new()),
                false_target: BlockTarget::new("bad".into(), Vec::new()),
            },
            operands: vec![ValueId::new("cond")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut ok_block = Block::new(Some("ok".into()), loc());
        ok_block.add_op(Operation {
            results: vec![ValueId::new("value")],
            kind: OperationKind::LoadElem,
            operands: vec![ValueId::new("xs"), ValueId::new("idx"), ValueId::new("pf")],
            result_types: vec![Type::new("i32")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        ok_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("value")],
            result_types: vec![Type::new("i32")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut bad_block = Block::new(Some("bad".into()), loc());
        bad_block.add_op(Operation {
            results: vec![ValueId::new("bad_value")],
            kind: OperationKind::LoadElem,
            operands: vec![ValueId::new("xs"), ValueId::new("idx"), ValueId::new("pf")],
            result_types: vec![Type::new("i32")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        bad_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("bad_value")],
            result_types: vec![Type::new("i32")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        func.add_block(entry);
        func.add_block(ok_block);
        func.add_block(bad_block);
        module.add_function(func).unwrap();

        let err = verify_module(&module)
            .expect_err("load_elem should reject proof that is unavailable on false branch");
        assert!(
            err.message
                .contains("arc.load_elem requires proof %pf to hold on all incoming paths"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn prove_and_refine_verify() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("idx"),
                ty: Type::new("index"),
                location: loc(),
            }],
            Some(Type::new("index")),
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
            results: vec![ValueId::new("assumed")],
            kind: OperationKind::Assume,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("!arc.proof<true>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("pf")],
            kind: OperationKind::Prove,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("!arc.proof<true>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("refined")],
            kind: OperationKind::Refine,
            operands: vec![ValueId::new("idx"), ValueId::new("pf")],
            result_types: vec![Type::new("index")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("refined")],
            result_types: vec![Type::new("index")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        verify_module(&module).expect("prove/refine should verify");
    }

    #[test]
    fn prove_requires_boolean_operand() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("idx"),
                ty: Type::new("index"),
                location: loc(),
            }],
            Some(Type::new("index")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("pf")],
            kind: OperationKind::Prove,
            operands: vec![ValueId::new("idx")],
            result_types: vec![Type::new("!arc.proof<true>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("idx")],
            result_types: vec![Type::new("index")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("prove operand must be boolean");
        assert!(
            err.message.contains("arc.prove operand must have type i1"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn prove_requires_established_fact() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(Symbol::new("main"), Vec::new(), Vec::new(), None, loc());
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
            kind: OperationKind::Prove,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("!arc.proof<true>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
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
        let err = verify_module(&module).expect_err("prove without establishing fact must fail");
        assert!(
            err.message.contains(
                "arc.prove operand %cond is not established as true on all incoming paths"
            ),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn prove_in_true_branch_verifies() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(Symbol::new("main"), Vec::new(), Vec::new(), None, loc());

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
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target: BlockTarget::new("then".into(), Vec::new()),
                false_target: BlockTarget::new("else".into(), Vec::new()),
            },
            operands: vec![ValueId::new("cond")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_op(Operation {
            results: vec![ValueId::new("pf")],
            kind: OperationKind::Prove,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("!arc.proof<true>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        then_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: Vec::new(),
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut else_block = Block::new(Some("else".into()), loc());
        else_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: Vec::new(),
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        func.add_block(entry);
        func.add_block(then_block);
        func.add_block(else_block);
        module.add_function(func).unwrap();
        verify_module(&module).expect("prove in true branch should verify");
    }

    #[test]
    fn prove_after_join_without_fact_is_error() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(Symbol::new("main"), Vec::new(), Vec::new(), None, loc());

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
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target: BlockTarget::new("then".into(), Vec::new()),
                false_target: BlockTarget::new("else".into(), Vec::new()),
            },
            operands: vec![ValueId::new("cond")],
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Branch {
                target: BlockTarget::new("join".into(), Vec::new()),
            },
            operands: Vec::new(),
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut else_block = Block::new(Some("else".into()), loc());
        else_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Branch {
                target: BlockTarget::new("join".into(), Vec::new()),
            },
            operands: Vec::new(),
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        let mut join_block = Block::new(Some("join".into()), loc());
        join_block.add_op(Operation {
            results: vec![ValueId::new("pf")],
            kind: OperationKind::Prove,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("!arc.proof<true>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        join_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: Vec::new(),
            result_types: Vec::new(),
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });

        func.add_block(entry);
        func.add_block(then_block);
        func.add_block(else_block);
        func.add_block(join_block);
        module.add_function(func).unwrap();
        let err = verify_module(&module)
            .expect_err("prove after join without fact must fail verification");
        assert!(
            err.message.contains(
                "arc.prove operand %cond is not established as true on all incoming paths"
            ),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn refine_allows_constructor_preserving_change() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("ptr"),
                ty: Type::new("!arc.ptr<i64>"),
                location: loc(),
            }],
            Some(Type::new("!arc.ptr<i64, align = 8>")),
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
            results: vec![ValueId::new("refined")],
            kind: OperationKind::Refine,
            operands: vec![ValueId::new("ptr"), ValueId::new("pf")],
            result_types: vec![Type::new("!arc.ptr<i64, align = 8>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("refined")],
            result_types: vec![Type::new("!arc.ptr<i64, align = 8>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        verify_module(&module).expect("constructor-preserving refine should verify");
    }

    #[test]
    fn refine_requires_constructor_match() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("ptr"),
                ty: Type::new("!arc.ptr<i64>"),
                location: loc(),
            }],
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
            results: vec![ValueId::new("refined")],
            kind: OperationKind::Refine,
            operands: vec![ValueId::new("ptr"), ValueId::new("pf")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("refined")],
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        let err = verify_module(&module)
            .expect_err("refine to mismatched constructor should fail verification");
        assert!(
            err.message
                .contains("arc.refine result type must share constructor with value type"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn refine_requires_proof_operand() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![Argument {
                name: ValueId::new("idx"),
                ty: Type::new("index"),
                location: loc(),
            }],
            Some(Type::new("index")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("refined")],
            kind: OperationKind::Refine,
            operands: vec![ValueId::new("idx"), ValueId::new("idx")],
            result_types: vec![Type::new("index")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("idx")],
            result_types: vec![Type::new("index")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("refine requires proof operand");
        assert!(
            err.message
                .contains("arc.refine second operand must be proof type"),
            "unexpected error: {}",
            err
        );
    }

    // --- Extended verification tests ---

    fn simple_valid_module() -> Module {
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
            result_types: vec![],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        module
    }

    #[test]
    fn extended_verify_clean_module() {
        let module = simple_valid_module();
        let ctx = arc_security::SecurityContext::new();
        let result = verify_module_extended(&module, &ctx, None).unwrap();
        assert!(result.is_clean(), "simple module should be clean");
        assert_eq!(result.issue_count(), 0);
    }

    #[test]
    fn extended_verify_reports_security_audit() {
        let module = simple_valid_module();
        let ctx = arc_security::SecurityContext::new();
        let result = verify_module_extended(&module, &ctx, None).unwrap();
        // Audit should have examined the module (even if no violations)
        assert_eq!(result.audit.capability_invocations, 0);
    }

    #[test]
    fn extended_verify_detects_memory_violations() {
        // A module with alloc + load (without store) — uninitialized read
        let mut module = Module::new(Symbol::new("test"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            vec![
                Argument {
                    name: ValueId::new("mem"),
                    ty: Type::new("!arc.mem"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("size"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("!arc.mem")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        // alloc: %mem, %size -> %mem1, %ptr
        entry.add_op(Operation {
            results: vec![ValueId::new("mem1"), ValueId::new("ptr")],
            kind: OperationKind::Alloc,
            operands: vec![ValueId::new("mem"), ValueId::new("size")],
            result_types: vec![Type::new("!arc.mem"), Type::new("!arc.ptr<i64>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        // load without prior store — should be uninit read
        entry.add_op(Operation {
            results: vec![ValueId::new("mem2"), ValueId::new("val")],
            kind: OperationKind::Load,
            operands: vec![ValueId::new("mem1"), ValueId::new("ptr")],
            result_types: vec![Type::new("!arc.mem"), Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("mem2")],
            result_types: vec![Type::new("!arc.mem")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let ctx = arc_security::SecurityContext::new();
        let result = verify_module_extended(&module, &ctx, None).unwrap();
        // Memory analysis should detect the uninitialized read
        assert!(
            !result.memory_violations.is_empty(),
            "should detect uninitialized read"
        );
    }

    #[test]
    fn extended_verify_fails_on_base_error() {
        // Module that fails base verification (empty function)
        let mut module = Module::new(Symbol::new("test"));
        let func = Function::new(Symbol::new("bad"), Vec::new(), Vec::new(), None, loc());
        module.add_function(func).unwrap();

        let ctx = arc_security::SecurityContext::new();
        assert!(
            verify_module_extended(&module, &ctx, None).is_err(),
            "should fail on base verification"
        );
    }

    #[test]
    fn extended_verify_sandbox_violation() {
        // Module with capability invocation, sandboxed to block it
        let mut module = Module::new(Symbol::new("test"));
        module
            .add_capability(arc_ir::Capability {
                name: Symbol::new("net.fetch"),
                inputs: vec![Argument {
                    name: ValueId::new("url"),
                    ty: Type::new("i64"),
                    location: loc(),
                }],
                outputs: vec![Argument {
                    name: ValueId::new("data"),
                    ty: Type::new("i64"),
                    location: loc(),
                }],
                effects: vec!["network".to_string()],
                failures: vec![],
                location: loc(),
            })
            .unwrap();

        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("u")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("auth")],
            kind: OperationKind::RequireApproval,
            operands: vec![ValueId::new("u"), ValueId::new("u")],
            result_types: vec![Type::new("!arc.auth<net.fetch>")],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("result")],
            kind: OperationKind::Invoke {
                capability: Symbol::new("net.fetch"),
            },
            operands: vec![ValueId::new("u")],
            result_types: vec![Type::new("i64")],
            effects: vec!["network".to_string()],
            location: loc(),
            regions: vec![],
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("result")],
            result_types: vec![],
            effects: Vec::new(),
            location: loc(),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let ctx = arc_security::SecurityContext::new();
        let mut sandbox = arc_security::SandboxPolicy::new(arc_security::SecurityLevel::Public);
        sandbox.deny("net.fetch");

        let result = verify_module_extended(&module, &ctx, Some(&sandbox)).unwrap();
        assert!(
            !result.security_violations.is_empty(),
            "should detect sandbox violation for denied capability"
        );
    }
}
