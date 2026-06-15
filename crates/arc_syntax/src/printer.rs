use arc_ir::{BlockTarget, IcmpPredicate, Module, Operation, OperationKind};
use std::fmt::Write;

pub fn print_module(module: &Module) -> String {
    let mut out = String::new();
    writeln!(out, "arc.module @{} {{", module.name.as_str()).unwrap();
    for cap in module.capabilities.values() {
        print_capability(cap, &mut out);
    }
    for func in module.functions.values() {
        print_function(func, &mut out);
    }
    writeln!(out, "}}").unwrap();
    out
}

fn print_capability(cap: &arc_ir::Capability, out: &mut String) {
    writeln!(out, "  arc.capability @{} {{", cap.name.as_str()).unwrap();
    if !cap.inputs.is_empty() {
        write!(out, "    inputs(").unwrap();
        for (i, input) in cap.inputs.iter().enumerate() {
            if i > 0 {
                write!(out, ", ").unwrap();
            }
            write!(out, "%{}: {}", input.name.as_str(), input.ty.as_str()).unwrap();
        }
        writeln!(out, ")").unwrap();
    }
    if !cap.outputs.is_empty() {
        write!(out, "    outputs(").unwrap();
        for (i, output) in cap.outputs.iter().enumerate() {
            if i > 0 {
                write!(out, ", ").unwrap();
            }
            write!(out, "%{}: {}", output.name.as_str(), output.ty.as_str()).unwrap();
        }
        writeln!(out, ")").unwrap();
    }
    if !cap.effects.is_empty() {
        write!(out, "    effects [").unwrap();
        for (i, eff) in cap.effects.iter().enumerate() {
            if i > 0 {
                write!(out, ", ").unwrap();
            }
            write!(out, "#arc.effect<{}>", eff).unwrap();
        }
        writeln!(out, "]").unwrap();
    }
    if !cap.failures.is_empty() {
        write!(out, "    failures [").unwrap();
        for (i, fail) in cap.failures.iter().enumerate() {
            if i > 0 {
                write!(out, ", ").unwrap();
            }
            write!(out, "#arc.fail<{}>", fail).unwrap();
        }
        writeln!(out, "]").unwrap();
    }
    writeln!(out, "  }}").unwrap();
}

fn print_function(func: &arc_ir::Function, out: &mut String) {
    write!(out, "  arc.func @{}", func.name.as_str()).unwrap();
    if !func.index_params.is_empty() {
        write!(out, " forall (").unwrap();
        for (i, param) in func.index_params.iter().enumerate() {
            if i > 0 {
                write!(out, ", ").unwrap();
            }
            write!(out, "%{}: index", param.name.as_str()).unwrap();
        }
        write!(out, ")").unwrap();
    }
    write!(out, "(").unwrap();
    for (i, param) in func.params.iter().enumerate() {
        if i > 0 {
            write!(out, ", ").unwrap();
        }
        write!(out, "%{}: {}", param.name.as_str(), param.ty.as_str()).unwrap();
    }
    write!(out, ")").unwrap();
    if let Some(result_ty) = &func.result {
        write!(out, " -> {}", result_ty.as_str()).unwrap();
    }
    writeln!(out, " {{").unwrap();
    for block in &func.blocks {
        print_block(block, out);
    }
    writeln!(out, "  }}").unwrap();
}

fn print_block(block: &arc_ir::Block, out: &mut String) {
    write!(
        out,
        "  ^{}",
        block.label().map(|l| l.as_str()).unwrap_or("bb")
    )
    .unwrap();
    if !block.args.is_empty() {
        write!(out, "(").unwrap();
        for (i, arg) in block.args.iter().enumerate() {
            if i > 0 {
                write!(out, ", ").unwrap();
            }
            write!(out, "%{}: {}", arg.name.as_str(), arg.ty.as_str()).unwrap();
        }
        write!(out, ")").unwrap();
    }
    writeln!(out, ":").unwrap();
    for op in &block.ops {
        print_operation(op, out);
    }
}

fn print_operation(op: &Operation, out: &mut String) {
    write!(out, "    ").unwrap();
    match &op.kind {
        OperationKind::ConstI64(value) => {
            write!(
                out,
                "%{} = arc.const {} : {}",
                op.results[0].as_str(),
                value,
                op.result_types[0].as_str()
            )
            .unwrap();
        }
        OperationKind::Add | OperationKind::Sub | OperationKind::Mul | OperationKind::Div => {
            let op_name = match &op.kind {
                OperationKind::Add => "arc.add",
                OperationKind::Sub => "arc.sub",
                OperationKind::Mul => "arc.mul",
                OperationKind::Div => "arc.div",
                _ => unreachable!(),
            };
            write!(
                out,
                "%{} = {} %{}, %{} : {}",
                op.results[0].as_str(),
                op_name,
                op.operands[0].as_str(),
                op.operands[1].as_str(),
                op.result_types[0].as_str()
            )
            .unwrap();
        }
        OperationKind::ICmp { predicate } => {
            let pred_str = match predicate {
                IcmpPredicate::Eq => "eq",
                IcmpPredicate::Ne => "ne",
                IcmpPredicate::Slt => "slt",
                IcmpPredicate::Sle => "sle",
                IcmpPredicate::Sgt => "sgt",
                IcmpPredicate::Sge => "sge",
            };
            write!(
                out,
                "%{} = arc.icmp {} %{}, %{} : {}",
                op.results[0].as_str(),
                pred_str,
                op.operands[0].as_str(),
                op.operands[1].as_str(),
                op.result_types[0].as_str()
            )
            .unwrap();
        }
        OperationKind::Alloc => {
            print_results(&op.results, out);
            write!(out, " = arc.alloc ").unwrap();
            print_operands(&op.operands, out);
            write!(out, " : ").unwrap();
            print_func_type(&op.result_types, out);
        }
        OperationKind::Load => {
            print_results(&op.results, out);
            write!(out, " = arc.load ").unwrap();
            print_operands(&op.operands, out);
            write!(out, " : ").unwrap();
            print_func_type(&op.result_types, out);
        }
        OperationKind::Store => {
            print_results(&op.results, out);
            write!(out, " = arc.store ").unwrap();
            print_operands(&op.operands, out);
            write!(out, " : ").unwrap();
            print_func_type(&op.result_types, out);
        }
        OperationKind::LoadElem => {
            let slice = &op.operands[0];
            let index = &op.operands[1];
            write!(
                out,
                "%{} = arc.load_elem %{}[%{}]",
                op.results[0].as_str(),
                slice.as_str(),
                index.as_str()
            )
            .unwrap();
            if op.operands.len() > 2 {
                write!(out, " requires ").unwrap();
                for (i, pf) in op.operands[2..].iter().enumerate() {
                    if i > 0 {
                        write!(out, ", ").unwrap();
                    }
                    write!(out, "%{}", pf.as_str()).unwrap();
                }
            }
            if !op.result_types.is_empty() {
                write!(out, " : ").unwrap();
                print_func_type(&op.result_types, out);
            }
        }
        OperationKind::Assume => {
            write!(
                out,
                "%{} = arc.assume %{} : {}",
                op.results[0].as_str(),
                op.operands[0].as_str(),
                op.result_types[0].as_str()
            )
            .unwrap();
        }
        OperationKind::Assert => {
            write!(out, "arc.assert %{}", op.operands[0].as_str()).unwrap();
            if let Some(ty) = op.result_types.first() {
                write!(out, " : {}", ty.as_str()).unwrap();
            }
        }
        OperationKind::Prove => {
            write!(
                out,
                "%{} = arc.prove %{} : {}",
                op.results[0].as_str(),
                op.operands[0].as_str(),
                op.result_types[0].as_str()
            )
            .unwrap();
        }
        OperationKind::Refine => {
            write!(
                out,
                "%{} = arc.refine %{}, %{} : {}",
                op.results[0].as_str(),
                op.operands[0].as_str(),
                op.operands[1].as_str(),
                op.result_types[0].as_str()
            )
            .unwrap();
        }
        OperationKind::Branch { target } => {
            write!(out, "arc.br ").unwrap();
            print_block_target(target, out);
        }
        OperationKind::CondBranch {
            true_target,
            false_target,
        } => {
            write!(out, "arc.cond_br %{}, ", op.operands[0].as_str()).unwrap();
            print_block_target(true_target, out);
            write!(out, ", ").unwrap();
            print_block_target(false_target, out);
        }
        OperationKind::Call { callee } => {
            if !op.results.is_empty() {
                print_results(&op.results, out);
                write!(out, " = ").unwrap();
            }
            write!(out, "arc.call @{}(", callee.as_str()).unwrap();
            for (i, operand) in op.operands.iter().enumerate() {
                if i > 0 {
                    write!(out, ", ").unwrap();
                }
                write!(out, "%{}", operand.as_str()).unwrap();
            }
            write!(out, ")").unwrap();
        }
        OperationKind::RequireApproval => {
            write!(
                out,
                "%{} = arc.require_approval %{}, %{} : {}",
                op.results[0].as_str(),
                op.operands[0].as_str(),
                op.operands[1].as_str(),
                op.result_types[0].as_str()
            )
            .unwrap();
        }
        OperationKind::Invoke { capability } => {
            if !op.results.is_empty() {
                print_results(&op.results, out);
                write!(out, " = ").unwrap();
            }
            write!(out, "arc.invoke @{}(", capability.as_str()).unwrap();
            for (i, operand) in op.operands.iter().enumerate() {
                if i > 0 {
                    write!(out, ", ").unwrap();
                }
                write!(out, "%{}", operand.as_str()).unwrap();
            }
            write!(out, ")").unwrap();
            if !op.result_types.is_empty() {
                write!(out, " : ").unwrap();
                print_func_type(&op.result_types, out);
            }
        }
        OperationKind::Return => {
            write!(out, "arc.return").unwrap();
            if let Some(value) = op.operands.first() {
                write!(out, " %{}", value.as_str()).unwrap();
            }
            if let Some(ty) = op.result_types.first() {
                write!(out, " : {}", ty.as_str()).unwrap();
            }
        }
        OperationKind::If => {
            if !op.results.is_empty() {
                print_results(&op.results, out);
                write!(out, " = ").unwrap();
            }
            write!(out, "arc.if %{}", op.operands[0].as_str()).unwrap();
            if !op.regions.is_empty() {
                writeln!(out, " {{").unwrap();
                for block in &op.regions[0].blocks {
                    print_block(block, out);
                }
                write!(out, "    }}").unwrap();
            }
            if op.regions.len() >= 2 {
                writeln!(out, " else {{").unwrap();
                for block in &op.regions[1].blocks {
                    print_block(block, out);
                }
                write!(out, "    }}").unwrap();
            }
        }
        OperationKind::Loop { iter_args } => {
            if !op.results.is_empty() {
                print_results(&op.results, out);
                write!(out, " = ").unwrap();
            }
            write!(out, "arc.loop").unwrap();
            if !iter_args.is_empty() {
                write!(out, " iter_args(").unwrap();
                for (i, arg) in iter_args.iter().enumerate() {
                    if i > 0 {
                        write!(out, ", ").unwrap();
                    }
                    write!(out, "%{}", arg.as_str()).unwrap();
                }
                write!(out, ")").unwrap();
            }
            if !op.regions.is_empty() {
                writeln!(out, " {{").unwrap();
                for block in &op.regions[0].blocks {
                    print_block(block, out);
                }
                write!(out, "    }}").unwrap();
            }
        }
        OperationKind::Yield => {
            write!(out, "arc.yield").unwrap();
            if !op.operands.is_empty() {
                write!(out, " ").unwrap();
                print_operands(&op.operands, out);
            }
        }
        OperationKind::Spawn { callee } => {
            if let Some(result) = op.results.first() {
                write!(out, "%{} = ", result.as_str()).unwrap();
            }
            write!(out, "arc.spawn @{}(", callee.as_str()).unwrap();
            for (i, operand) in op.operands.iter().enumerate() {
                if i > 0 {
                    write!(out, ", ").unwrap();
                }
                write!(out, "%{}", operand.as_str()).unwrap();
            }
            write!(out, ")").unwrap();
        }
        OperationKind::Await => {
            if let Some(result) = op.results.first() {
                write!(out, "%{} = ", result.as_str()).unwrap();
            }
            write!(out, "arc.await %{}", op.operands[0].as_str()).unwrap();
        }
        OperationKind::Checkpoint { label } => {
            if let Some(result) = op.results.first() {
                write!(out, "%{} = ", result.as_str()).unwrap();
            }
            write!(out, "arc.checkpoint \"{}\"", label).unwrap();
        }
        OperationKind::Unknown(name) => {
            if !op.results.is_empty() {
                print_results(&op.results, out);
                write!(out, " = ").unwrap();
            }
            write!(out, "{}", name).unwrap();
            if !op.operands.is_empty() {
                write!(out, " ").unwrap();
                print_operands(&op.operands, out);
            }
        }
    }
    writeln!(out).unwrap();
}

fn print_results(results: &[arc_ir::ValueId], out: &mut String) {
    for (i, r) in results.iter().enumerate() {
        if i > 0 {
            write!(out, ", ").unwrap();
        }
        write!(out, "%{}", r.as_str()).unwrap();
    }
}

fn print_operands(operands: &[arc_ir::ValueId], out: &mut String) {
    for (i, o) in operands.iter().enumerate() {
        if i > 0 {
            write!(out, ", ").unwrap();
        }
        write!(out, "%{}", o.as_str()).unwrap();
    }
}

fn print_block_target(target: &BlockTarget, out: &mut String) {
    write!(out, "^{}", target.label).unwrap();
    if !target.arguments.is_empty() {
        write!(out, "(").unwrap();
        for (i, arg) in target.arguments.iter().enumerate() {
            if i > 0 {
                write!(out, ", ").unwrap();
            }
            write!(out, "%{}", arg.as_str()).unwrap();
        }
        write!(out, ")").unwrap();
    }
}

fn print_func_type(result_types: &[arc_ir::Type], out: &mut String) {
    if result_types.len() == 1 {
        write!(out, "{}", result_types[0].as_str()).unwrap();
    } else {
        write!(out, "(").unwrap();
        for (i, ty) in result_types.iter().enumerate() {
            if i > 0 {
                write!(out, ", ").unwrap();
            }
            write!(out, "{}", ty.as_str()).unwrap();
        }
        write!(out, ")").unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_module;

    #[test]
    fn round_trip_simple_module() {
        let source = "\
arc.module @m {
  arc.func @add(%a: i64, %b: i64) -> i64 {
  ^entry:
    %c = arc.add %a, %b : i64
    arc.return %c : i64
  }
}
";
        let module = parse_module(source).expect("parse");
        let printed = print_module(&module);
        let reparsed = parse_module(&printed).expect("reparse");
        let reprinted = print_module(&reparsed);
        assert_eq!(printed, reprinted, "round-trip unstable");
    }

    #[test]
    fn round_trip_branching() {
        let source = "\
arc.module @m {
  arc.func @f(%x: i64) -> i64 {
  ^entry:
    %zero = arc.const 0 : i64
    %cond = arc.icmp eq %x, %zero : i1
    arc.cond_br %cond, ^then, ^else
  ^then:
    %one = arc.const 1 : i64
    arc.return %one : i64
  ^else:
    arc.return %x : i64
  }
}
";
        let module = parse_module(source).expect("parse");
        let printed = print_module(&module);
        let reparsed = parse_module(&printed).expect("reparse");
        let reprinted = print_module(&reparsed);
        assert_eq!(printed, reprinted, "round-trip unstable");
    }

    #[test]
    fn round_trip_call() {
        let source = "\
arc.module @m {
  arc.func @double(%x: i64) -> i64 {
  ^entry:
    %r = arc.add %x, %x : i64
    arc.return %r : i64
  }
  arc.func @main() -> i64 {
  ^entry:
    %a = arc.const 5 : i64
    %b = arc.call @double(%a) : (i64) -> i64
    arc.return %b : i64
  }
}
";
        let module = parse_module(source).expect("parse");
        let printed = print_module(&module);
        let reparsed = parse_module(&printed).expect("reparse");
        let reprinted = print_module(&reparsed);
        assert_eq!(printed, reprinted, "round-trip unstable");
    }
}
