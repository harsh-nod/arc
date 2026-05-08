use air_ir::{Module, OperationKind, ValueId};
use std::collections::HashMap;
use thiserror::Error;

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
        .get(&air_ir::Symbol::new("main"))
        .ok_or_else(|| InterpreterError::new("module missing @main function"))?;
    if main.blocks.is_empty() {
        return Err(InterpreterError::new("@main has no blocks"));
    }
    if !main.params.is_empty() {
        return Err(InterpreterError::new(format!(
            "@main expects no parameters but found {}",
            main.params.len()
        )));
    }

    let mut values: HashMap<String, i64> = HashMap::new();
    let mut current_block = 0usize;
    let mut incoming_args: Vec<i64> = Vec::new();
    let mut steps = 0usize;

    loop {
        steps += 1;
        if steps > 100_000 {
            return Err(InterpreterError::new(
                "interpreter exceeded step limit (possible infinite loop)",
            ));
        }

        let block = main
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
            values.insert(arg.name.as_str().to_string(), value);
        }

        let mut advanced_control_flow = false;
        for op in &block.ops {
            match &op.kind {
                OperationKind::ConstI64(value) => {
                    let result = op
                        .results
                        .get(0)
                        .ok_or_else(|| InterpreterError::new("const missing result"))?;
                    values.insert(result.as_str().to_string(), *value);
                }
                OperationKind::Add => {
                    let lhs = read_value(&values, &op.operands[0])?;
                    let rhs = read_value(&values, &op.operands[1])?;
                    let result = op
                        .results
                        .get(0)
                        .ok_or_else(|| InterpreterError::new("add missing result"))?;
                    values.insert(result.as_str().to_string(), lhs + rhs);
                }
                OperationKind::Sub => {
                    let lhs = read_value(&values, &op.operands[0])?;
                    let rhs = read_value(&values, &op.operands[1])?;
                    let result = op
                        .results
                        .get(0)
                        .ok_or_else(|| InterpreterError::new("sub missing result"))?;
                    values.insert(result.as_str().to_string(), lhs - rhs);
                }
                OperationKind::Mul => {
                    let lhs = read_value(&values, &op.operands[0])?;
                    let rhs = read_value(&values, &op.operands[1])?;
                    let result = op
                        .results
                        .get(0)
                        .ok_or_else(|| InterpreterError::new("mul missing result"))?;
                    values.insert(result.as_str().to_string(), lhs * rhs);
                }
                OperationKind::Div => {
                    let lhs = read_value(&values, &op.operands[0])?;
                    let rhs = read_value(&values, &op.operands[1])?;
                    if rhs == 0 {
                        return Err(InterpreterError::new("division by zero"));
                    }
                    let result = op
                        .results
                        .get(0)
                        .ok_or_else(|| InterpreterError::new("div missing result"))?;
                    values.insert(result.as_str().to_string(), lhs / rhs);
                }
                OperationKind::ICmp { predicate } => {
                    let lhs = read_value(&values, &op.operands[0])?;
                    let rhs = read_value(&values, &op.operands[1])?;
                    let result = op
                        .results
                        .get(0)
                        .ok_or_else(|| InterpreterError::new("icmp missing result"))?;
                    let value = match predicate {
                        air_ir::IcmpPredicate::Eq => (lhs == rhs) as i64,
                        air_ir::IcmpPredicate::Ne => (lhs != rhs) as i64,
                        air_ir::IcmpPredicate::Slt => (lhs < rhs) as i64,
                        air_ir::IcmpPredicate::Sle => (lhs <= rhs) as i64,
                        air_ir::IcmpPredicate::Sgt => (lhs > rhs) as i64,
                        air_ir::IcmpPredicate::Sge => (lhs >= rhs) as i64,
                    };
                    values.insert(result.as_str().to_string(), value);
                }
                OperationKind::Branch { target } => {
                    let (next_idx, args) = resolve_branch_target(main, target, &values)?;
                    current_block = next_idx;
                    incoming_args = args;
                    advanced_control_flow = true;
                    break;
                }
                OperationKind::CondBranch {
                    true_target,
                    false_target,
                } => {
                    let cond_value = read_value(&values, &op.operands[0])?;
                    let target = if cond_value != 0 {
                        true_target
                    } else {
                        false_target
                    };
                    let (next_idx, args) = resolve_branch_target(main, target, &values)?;
                    current_block = next_idx;
                    incoming_args = args;
                    advanced_control_flow = true;
                    break;
                }
                OperationKind::Alloc | OperationKind::Load | OperationKind::Store => {
                    return Err(InterpreterError::new(
                        "memory operations are not supported by the interpreter yet",
                    ));
                }
                OperationKind::Return => {
                    if let Some(value) = op.operands.first() {
                        let result = read_value(&values, value)?;
                        return Ok(Some(result));
                    } else {
                        return Ok(None);
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
    function: &air_ir::Function,
    target: &air_ir::BlockTarget,
    values: &HashMap<String, i64>,
) -> Result<(usize, Vec<i64>), InterpreterError> {
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

fn read_value(values: &HashMap<String, i64>, value: &ValueId) -> Result<i64, InterpreterError> {
    values
        .get(value.as_str())
        .cloned()
        .ok_or_else(|| InterpreterError::new(format!("value {} not defined", value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use air_ir::{
        Block, BlockTarget, Function, IcmpPredicate, Location, Module, Operation, OperationKind,
        Symbol, Type, ValueId,
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
            Some(Type::new("i64")),
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
            results: vec![ValueId::new("one")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: vec![ValueId::new("cond")],
            kind: OperationKind::ICmp {
                predicate: IcmpPredicate::Eq,
            },
            operands: vec![ValueId::new("zero"), ValueId::new("one")],
            result_types: vec![Type::new("i1")],
            location: loc(),
        });
        entry.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::CondBranch {
                true_target: BlockTarget::new("then".into(), vec![]),
                false_target: BlockTarget::new("else".into(), vec![]),
            },
            operands: vec![ValueId::new("cond")],
            result_types: Vec::new(),
            location: loc(),
        });

        let mut then_block = Block::new(Some("then".into()), loc());
        then_block.add_op(Operation {
            results: vec![ValueId::new("value")],
            kind: OperationKind::ConstI64(1),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            location: loc(),
        });
        then_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("value")],
            result_types: vec![Type::new("i64")],
            location: loc(),
        });

        let mut else_block = Block::new(Some("else".into()), loc());
        else_block.add_op(Operation {
            results: vec![ValueId::new("value2")],
            kind: OperationKind::ConstI64(2),
            operands: Vec::new(),
            result_types: vec![Type::new("i64")],
            location: loc(),
        });
        else_block.add_op(Operation {
            results: Vec::new(),
            kind: OperationKind::Return,
            operands: vec![ValueId::new("value2")],
            result_types: vec![Type::new("i64")],
            location: loc(),
        });

        func.add_block(entry);
        func.add_block(then_block);
        func.add_block(else_block);
        module.add_function(func).unwrap();

        let result = run_main(&module).expect("interpreter succeeded");
        assert_eq!(result, Some(2));
    }
}
