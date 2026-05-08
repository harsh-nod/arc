use air_ir::{Function, Location, Module, OperationKind, Type, ValueId};
use std::collections::{HashMap, HashSet, VecDeque};

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
        verify_function(func)?;
    }

    Ok(())
}

fn verify_function(func: &Function) -> Result<(), VerifyError> {
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
                            "air.alloc expects memory and size operands",
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
                    if !is_resource_type(&mem_ty) || mem_ty.as_str() != "!air.mem" {
                        return Err(VerifyError::new(
                            "air.alloc first operand must be !air.mem resource",
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
                            "air.alloc size operand must have type index, found {}",
                            size_ty.as_str()
                        )));
                    }
                    if op.results.len() != 2 || op.result_types.len() != 2 {
                        return Err(VerifyError::new(
                            "air.alloc must produce updated memory and pointer results",
                        ));
                    }
                    let new_mem_ty = op.result_types[0].clone();
                    if new_mem_ty != mem_ty {
                        return Err(VerifyError::new(format!(
                            "air.alloc must return updated memory of type {}, found {}",
                            mem_ty.as_str(),
                            new_mem_ty.as_str()
                        )));
                    }
                    let ptr_ty = op.result_types[1].clone();
                    if !is_pointer_type(&ptr_ty) {
                        return Err(VerifyError::new(format!(
                            "air.alloc second result must be pointer type, found {}",
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
                            "air.load expects memory and pointer operands",
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
                    if mem_ty.as_str() != "!air.mem" {
                        return Err(VerifyError::new(format!(
                            "air.load first operand must be !air.mem, found {}",
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
                            "air.load pointer operand must be pointer type, found {}",
                            ptr_ty.as_str()
                        )));
                    }
                    if op.results.len() != 2 || op.result_types.len() != 2 {
                        return Err(VerifyError::new(
                            "air.load must return updated memory and loaded value",
                        ));
                    }
                    let new_mem_ty = op.result_types[0].clone();
                    if new_mem_ty != mem_ty {
                        return Err(VerifyError::new(format!(
                            "air.load must return memory of type {}, found {}",
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
                OperationKind::Store => {
                    if op.operands.len() != 3 {
                        return Err(VerifyError::new(
                            "air.store expects memory, pointer, and value operands",
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
                    if mem_ty.as_str() != "!air.mem" {
                        return Err(VerifyError::new(format!(
                            "air.store first operand must be !air.mem, found {}",
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
                            "air.store pointer operand must be pointer type, found {}",
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
                            "air.store must return updated memory resource",
                        ));
                    }
                    let new_mem_ty = op.result_types[0].clone();
                    if new_mem_ty != mem_ty {
                        return Err(VerifyError::new(format!(
                            "air.store must return memory of type {}, found {}",
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
                            "air.assume expects exactly one condition operand",
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
                            "air.assume condition must have type i1, found {}",
                            cond_ty.as_str()
                        )));
                    }
                    if op.results.len() != 1 || op.result_types.len() != 1 {
                        return Err(VerifyError::new(
                            "air.assume must produce exactly one proof result",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    let proof_ty = op.result_types[0].clone();
                    if !is_proof_type(&proof_ty) {
                        return Err(VerifyError::new(format!(
                            "air.assume result must be proof type, found {}",
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
                OperationKind::Assert => {
                    if !op.results.is_empty() {
                        return Err(VerifyError::new("air.assert does not produce results"));
                    }
                    if op.operands.len() != 1 {
                        return Err(VerifyError::new(
                            "air.assert expects exactly one condition operand",
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
                            "air.assert condition must have type i1, found {}",
                            cond_ty.as_str()
                        )));
                    }
                    if op.result_types.len() > 1 {
                        return Err(VerifyError::new(
                            "air.assert may have at most one type annotation",
                        ));
                    }
                    validate_type_list(&op.result_types, &index_params)?;
                    if let Some(annotation) = op.result_types.first() {
                        if annotation != &cond_ty {
                            return Err(VerifyError::new(format!(
                                "air.assert annotation must match operand type (expected {}, found {})",
                                cond_ty.as_str(),
                                annotation.as_str()
                            )));
                        }
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

fn verify_branch_target(
    func: &Function,
    definitions: &HashMap<String, Definition>,
    dominators: &[Vec<bool>],
    current_block: usize,
    target: &air_ir::BlockTarget,
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

fn is_resource_type(ty: &Type) -> bool {
    const RESOURCE_PREFIXES: &[&str] = &[
        "!air.mem",
        "!air.fs",
        "!air.net",
        "!air.db",
        "!air.world",
        "!air.clock",
        "!air.rng",
        "!air.ui",
        "!air.gpu",
        "!air.vault",
    ];
    let repr = ty.as_str();
    RESOURCE_PREFIXES
        .iter()
        .any(|prefix| repr.starts_with(prefix))
}

fn is_pointer_type(ty: &Type) -> bool {
    ty.as_str().starts_with("!air.ptr<") || ty.as_str() == "!air.ptr"
}

fn is_proof_type(ty: &Type) -> bool {
    let repr = ty.as_str();
    repr.starts_with("!air.proof<") || repr == "!air.proof"
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
                if (b'A'..=b'Z').contains(b)
                    || (b'a'..=b'z').contains(b)
                    || (b'0'..=b'9').contains(b)
                    || *b == b'_'
                {
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

#[cfg(test)]
mod tests {
    use super::*;
    use air_ir::{
        Argument, Block, IcmpPredicate, IndexParam, Location, Module, Operation, OperationKind,
        Symbol, Type, ValueId,
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
            location: loc(),
        });
        block.add_op(Operation {
            results: vec![ValueId::new("sum")],
            kind: OperationKind::Add,
            operands: vec![ValueId::new("one"), ValueId::new("one")],
            result_types: vec![Type::new("i64")],
            location: loc(),
        });
        block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("sum")],
            result_types: vec![Type::new("i64")],
            location: loc(),
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
            location: loc(),
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
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Branch {
                target: air_ir::BlockTarget::new("then".into(), vec![ValueId::new("x")]),
            },
            operands: Vec::new(),
            result_types: Vec::new(),
            location: loc(),
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
            location: loc(),
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
                target: air_ir::BlockTarget::new("then".into(), vec![]),
            },
            operands: Vec::new(),
            result_types: Vec::new(),
            location: loc(),
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
            location: loc(),
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
                ty: Type::new("!air.mem"),
                location: loc(),
            }],
            Some(Type::new("!air.mem")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("mem")],
            result_types: vec![Type::new("!air.mem")],
            location: loc(),
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
                ty: Type::new("!air.mem"),
                location: loc(),
            }],
            Some(Type::new("!air.mem")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("zero")],
            kind: OperationKind::ConstI64(0),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("always_true")],
            kind: OperationKind::ICmp {
                predicate: IcmpPredicate::Eq,
            },
            operands: vec![ValueId::new("zero"), ValueId::new("zero")],
            result_types: vec![Type::new("i1")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target: air_ir::BlockTarget::new("then".into(), vec![ValueId::new("mem")]),
                false_target: air_ir::BlockTarget::new("else".into(), vec![ValueId::new("mem")]),
            },
            operands: vec![ValueId::new("always_true")],
            result_types: Vec::new(),
            location: loc(),
        });
        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_arg(Argument {
            name: ValueId::new("m1"),
            ty: Type::new("!air.mem"),
            location: loc(),
        });
        then_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("m1")],
            result_types: vec![Type::new("!air.mem")],
            location: loc(),
        });
        let mut else_block = Block::new(Some("else".into()), loc());
        else_block.add_arg(Argument {
            name: ValueId::new("m2"),
            ty: Type::new("!air.mem"),
            location: loc(),
        });
        else_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("m2")],
            result_types: vec![Type::new("!air.mem")],
            location: loc(),
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
                    ty: Type::new("!air.mem"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("size"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("!air.mem")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("init")],
            kind: OperationKind::ConstI64(42),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("mem1"), ValueId::new("ptr")],
            kind: OperationKind::Alloc,
            operands: vec![ValueId::new("mem"), ValueId::new("size")],
            result_types: vec![Type::new("!air.mem"), Type::new("!air.ptr<i64>")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("mem2")],
            kind: OperationKind::Store,
            operands: vec![
                ValueId::new("mem1"),
                ValueId::new("ptr"),
                ValueId::new("init"),
            ],
            result_types: vec![Type::new("!air.mem")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("mem3"), ValueId::new("loaded")],
            kind: OperationKind::Load,
            operands: vec![ValueId::new("mem2"), ValueId::new("ptr")],
            result_types: vec![Type::new("!air.mem"), Type::new("i64")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("mem3")],
            result_types: vec![Type::new("!air.mem")],
            location: loc(),
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
                    ty: Type::new("!air.mem"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("size"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("!air.mem")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("mem1"), ValueId::new("not_ptr")],
            kind: OperationKind::Alloc,
            operands: vec![ValueId::new("mem"), ValueId::new("size")],
            result_types: vec![Type::new("!air.mem"), Type::new("i64")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("mem1")],
            result_types: vec![Type::new("!air.mem")],
            location: loc(),
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
                    ty: Type::new("!air.mem"),
                    location: loc(),
                },
                Argument {
                    name: ValueId::new("size"),
                    ty: Type::new("index"),
                    location: loc(),
                },
            ],
            Some(Type::new("!air.mem")),
            loc(),
        );
        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            results: vec![ValueId::new("val")],
            kind: OperationKind::ConstI64(0),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("mem1"), ValueId::new("ptr")],
            kind: OperationKind::Alloc,
            operands: vec![ValueId::new("mem"), ValueId::new("size")],
            result_types: vec![Type::new("!air.mem"), Type::new("!air.ptr<i8>")],
            location: loc(),
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
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("mem2")],
            result_types: vec![Type::new("!air.mem")],
            location: loc(),
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
                ty: Type::new("!air.slice<i32, %n>"),
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
            location: loc(),
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("zero")],
            kind: OperationKind::ConstI64(0),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("positive")],
            kind: OperationKind::ICmp {
                predicate: IcmpPredicate::Sgt,
            },
            operands: vec![ValueId::new("len"), ValueId::new("zero")],
            result_types: vec![Type::new("i1")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("pf")],
            kind: OperationKind::Assume,
            operands: vec![ValueId::new("positive")],
            result_types: vec![Type::new("!air.proof<%n > 0>")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("len")],
            result_types: vec![Type::new("i64")],
            location: loc(),
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
                ty: Type::new("!air.slice<i32, %n>"),
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
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("len")],
            result_types: vec![Type::new("i64")],
            location: loc(),
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
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("i64")],
            location: loc(),
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
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("value")],
            result_types: vec![Type::new("i64")],
            location: loc(),
        });
        func.add_block(entry);
        module.add_function(func).unwrap();
        let err = verify_module(&module).expect_err("assert should require boolean condition");
        assert!(
            err.message
                .contains("air.assert condition must have type i1"),
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
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target: air_ir::BlockTarget::new("then".into(), vec![]),
                false_target: air_ir::BlockTarget::new("else".into(), vec![]),
            },
            operands: vec![ValueId::new("cond_value")],
            result_types: Vec::new(),
            location: loc(),
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("cond_value")],
            result_types: vec![Type::new("i64")],
            location: loc(),
        });

        let mut else_block = Block::new(Some("else".into()), loc());
        else_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("cond_value")],
            result_types: vec![Type::new("i64")],
            location: loc(),
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
}
