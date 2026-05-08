use air_ir::{Function, Location, Module, OperationKind, Type, ValueId};
use std::collections::{HashMap, VecDeque};

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
                    let ty = op
                        .result_type
                        .clone()
                        .ok_or_else(|| VerifyError::new("const must declare a result type"))?;
                    let result = op
                        .result
                        .as_ref()
                        .ok_or_else(|| VerifyError::new("const must produce result"))?;
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
                    let result_ty = op
                        .result_type
                        .clone()
                        .ok_or_else(|| VerifyError::new("binary arithmetic must declare result"))?;
                    if result_ty != lhs_ty {
                        return Err(VerifyError::new(
                            "binary arithmetic result type must match operand type",
                        ));
                    }
                    let result = op
                        .result
                        .as_ref()
                        .ok_or_else(|| VerifyError::new("binary arithmetic missing result SSA"))?;
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
                    let result_ty = op
                        .result_type
                        .clone()
                        .ok_or_else(|| VerifyError::new("icmp must declare result type"))?;
                    if result_ty != Type::new("i1") {
                        return Err(VerifyError::new(format!(
                            "icmp result must have type i1, found {}",
                            result_ty.as_str()
                        )));
                    }
                    let result = op
                        .result
                        .as_ref()
                        .ok_or_else(|| VerifyError::new("icmp missing result SSA"))?;
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
                OperationKind::Return => {
                    let expected = func.result.as_ref();
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
                name,
                def.location.offset
            )));
        }
        if use_count > 1 {
            return Err(VerifyError::new(format!(
                "resource value %{} (defined at offset {}) used more than once",
                name,
                def.location.offset
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

#[cfg(test)]
mod tests {
    use super::*;
    use air_ir::{
        Argument, Block, IcmpPredicate, Location, Module, Operation, OperationKind, Symbol, Type,
        ValueId,
    };

    fn loc() -> Location {
        Location::new(0, 0)
    }

    fn minimal_function() -> Function {
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );
        let mut block = Block::new(Some("entry".into()), loc());
        block.add_op(Operation {
            result: Some(ValueId::new("one")),
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_type: Some(Type::new("i64")),
            location: loc(),
        });
        block.add_op(Operation {
            result: Some(ValueId::new("sum")),
            kind: OperationKind::Add,
            operands: vec![ValueId::new("one"), ValueId::new("one")],
            result_type: Some(Type::new("i64")),
            location: loc(),
        });
        block.add_op(Operation {
            result: None,
            kind: OperationKind::Return,
            operands: vec![ValueId::new("sum")],
            result_type: Some(Type::new("i64")),
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
            Some(Type::new("i64")),
            loc(),
        );
        let mut block = Block::new(Some("entry".into()), loc());
        block.add_op(Operation {
            result: Some(ValueId::new("sum")),
            kind: OperationKind::Add,
            operands: vec![ValueId::new("missing"), ValueId::new("missing")],
            result_type: Some(Type::new("i64")),
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
            Some(Type::new("i64")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            result: Some(ValueId::new("x")),
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_type: Some(Type::new("i64")),
            location: loc(),
        });
        entry.add_op(Operation {
            result: None,
            kind: OperationKind::Branch {
                target: air_ir::BlockTarget::new("then".into(), vec![ValueId::new("x")]),
            },
            operands: Vec::new(),
            result_type: None,
            location: loc(),
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_arg(Argument {
            name: ValueId::new("y"),
            ty: Type::new("i64"),
            location: loc(),
        });
        then_block.add_op(Operation {
            result: None,
            kind: OperationKind::Return,
            operands: vec![ValueId::new("y")],
            result_type: Some(Type::new("i64")),
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
            Some(Type::new("i64")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            result: None,
            kind: OperationKind::Branch {
                target: air_ir::BlockTarget::new("then".into(), vec![]),
            },
            operands: Vec::new(),
            result_type: None,
            location: loc(),
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_arg(Argument {
            name: ValueId::new("y"),
            ty: Type::new("i64"),
            location: loc(),
        });
        then_block.add_op(Operation {
            result: None,
            kind: OperationKind::Return,
            operands: vec![ValueId::new("y")],
            result_type: Some(Type::new("i64")),
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
            result: None,
            kind: OperationKind::Return,
            operands: vec![ValueId::new("mem")],
            result_type: Some(Type::new("!air.mem")),
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
            result: Some(ValueId::new("zero")),
            kind: OperationKind::ConstI64(0),
            operands: Vec::new(),
            result_type: Some(Type::new("i64")),
            location: loc(),
        });
        entry.add_op(Operation {
            result: Some(ValueId::new("always_true")),
            kind: OperationKind::ICmp {
                predicate: IcmpPredicate::Eq,
            },
            operands: vec![ValueId::new("zero"), ValueId::new("zero")],
            result_type: Some(Type::new("i1")),
            location: loc(),
        });
        entry.add_op(Operation {
            result: None,
            kind: OperationKind::CondBranch {
                true_target: air_ir::BlockTarget::new("then".into(), vec![ValueId::new("mem")]),
                false_target: air_ir::BlockTarget::new("else".into(), vec![ValueId::new("mem")]),
            },
            operands: vec![ValueId::new("always_true")],
            result_type: None,
            location: loc(),
        });
        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_arg(Argument {
            name: ValueId::new("m1"),
            ty: Type::new("!air.mem"),
            location: loc(),
        });
        then_block.add_op(Operation {
            result: None,
            kind: OperationKind::Return,
            operands: vec![ValueId::new("m1")],
            result_type: Some(Type::new("!air.mem")),
            location: loc(),
        });
        let mut else_block = Block::new(Some("else".into()), loc());
        else_block.add_arg(Argument {
            name: ValueId::new("m2"),
            ty: Type::new("!air.mem"),
            location: loc(),
        });
        else_block.add_op(Operation {
            result: None,
            kind: OperationKind::Return,
            operands: vec![ValueId::new("m2")],
            result_type: Some(Type::new("!air.mem")),
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
    fn cond_branch_requires_boolean_condition() {
        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("main"),
            Vec::new(),
            Some(Type::new("i64")),
            loc(),
        );

        let mut entry = Block::new(Some("entry".into()), loc());
        entry.add_op(Operation {
            result: Some(ValueId::new("cond_value")),
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_type: Some(Type::new("i64")),
            location: loc(),
        });
        entry.add_op(Operation {
            result: None,
            kind: OperationKind::CondBranch {
                true_target: air_ir::BlockTarget::new("then".into(), vec![]),
                false_target: air_ir::BlockTarget::new("else".into(), vec![]),
            },
            operands: vec![ValueId::new("cond_value")],
            result_type: None,
            location: loc(),
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_op(Operation {
            result: None,
            kind: OperationKind::Return,
            operands: vec![ValueId::new("cond_value")],
            result_type: Some(Type::new("i64")),
            location: loc(),
        });

        let mut else_block = Block::new(Some("else".into()), loc());
        else_block.add_op(Operation {
            result: None,
            kind: OperationKind::Return,
            operands: vec![ValueId::new("cond_value")],
            result_type: Some(Type::new("i64")),
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
