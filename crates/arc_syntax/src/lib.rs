mod parser;
mod printer;

pub use parser::{parse_module, ParseError};
pub use printer::print_module;

#[cfg(test)]
mod tests {
    use super::*;
    use arc_ir::{Function, Location, Module, Operation, OperationKind, Symbol, Type, ValueId};

    #[test]
    fn parse_valid_spec_example() {
        let source = r#"
arc.module @m {
  arc.func @add(%a: i64, %b: i64) -> i64 {
  ^entry:
    %c = arc.const 1 : i64
    %d = arc.add %a, %b : i64
    arc.return %d : i64
  }
}
"#;
        let module = parse_module(source).expect("module parses");
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn reject_missing_type() {
        let source = r#"
arc.module @bad {
  arc.func @add(%a: i64, %b: i64) {
  ^entry:
    %c = arc.add %a, %b
    arc.return %c
  }
}
"#;
        let err = parse_module(source).expect_err("parse should fail");
        assert!(
            err.message.contains("expected ':'"),
            "unexpected error: {}",
            err.message
        );
    }

    #[test]
    fn parse_conditional_branch() {
        let source = r#"
arc.module @m {
  arc.func @branch(%cond: i1, %x: i64) -> i64 {
  ^entry(%arg0: i64):
    arc.cond_br %cond, ^then(%arg0), ^else(%arg0)
  ^then(%y: i64):
    %one = arc.const 1 : i64
    %sum = arc.add %y, %one : i64
    arc.br ^exit(%sum)
  ^else(%z: i64):
    arc.br ^exit(%z)
  ^exit(%result: i64):
    arc.return %result : i64
  }
}
"#;
        let module = parse_module(source).expect("module parses");
        let func = module.functions.values().next().expect("function present");
        assert_eq!(func.blocks.len(), 4);
        let entry = &func.blocks[0];
        assert_eq!(entry.args.len(), 1);
        assert!(matches!(
            entry.ops.first().map(|op| &op.kind),
            Some(OperationKind::CondBranch { .. })
        ));
        let then_block = &func.blocks[1];
        assert!(matches!(
            then_block.ops.last().map(|op| &op.kind),
            Some(OperationKind::Branch { .. })
        ));
        let exit_block = &func.blocks[3];
        assert_eq!(exit_block.args.len(), 1);
        assert!(matches!(
            exit_block.ops.first().map(|op| &op.kind),
            Some(OperationKind::Return)
        ));
    }

    #[test]
    fn parse_function_with_forall_indices() {
        let source = r#"
arc.module @m {
  arc.func @f forall (%n: index, %m: index) (%xs: !arc.tensor<f32, [%n, %m]>) -> !arc.tensor<f32, [%n, %m]> {
  ^entry:
    arc.return %xs : !arc.tensor<f32, [%n, %m]>
  }
}
"#;
        let module = parse_module(source).expect("module parses");
        let func = module.functions.values().next().expect("function present");
        assert_eq!(func.index_params.len(), 2);
        assert_eq!(func.index_params[0].name.as_str(), "n");
        assert_eq!(func.index_params[1].name.as_str(), "m");
        assert_eq!(func.params.len(), 1);
        assert_eq!(func.params[0].ty.as_str(), "!arc.tensor<f32, [%n, %m]>");
        assert_eq!(
            func.result.as_ref().map(|ty| ty.as_str()),
            Some("!arc.tensor<f32, [%n, %m]>")
        );
    }

    #[test]
    fn parse_if_then_else() {
        let source = r#"
arc.module @m {
  arc.func @f(%cond: i1) -> i64 {
  ^entry:
    %r = arc.if %cond {
    ^then:
      %a = arc.const 1 : i64
      arc.yield %a
    } else {
    ^else_body:
      %b = arc.const 2 : i64
      arc.yield %b
    }
    arc.return %r : i64
  }
}
"#;
        let module = parse_module(source).expect("if/else parses");
        let func = module.functions.values().next().unwrap();
        let entry = &func.blocks[0];
        let if_op = &entry.ops[0];
        assert!(matches!(if_op.kind, OperationKind::If));
        assert_eq!(if_op.operands.len(), 1);
        assert_eq!(if_op.operands[0].as_str(), "cond");
        assert_eq!(if_op.regions.len(), 2);
        // then region has 1 block with 2 ops (const + yield)
        assert_eq!(if_op.regions[0].blocks.len(), 1);
        assert_eq!(if_op.regions[0].blocks[0].ops.len(), 2);
        // else region has 1 block with 2 ops
        assert_eq!(if_op.regions[1].blocks.len(), 1);
        assert_eq!(if_op.regions[1].blocks[0].ops.len(), 2);
    }

    #[test]
    fn parse_loop_with_yield() {
        let source = r#"
arc.module @m {
  arc.func @f() -> i64 {
  ^entry:
    %r = arc.loop iter_args(%i) {
    ^body(%counter: i64):
      %one = arc.const 1 : i64
      %next = arc.add %counter, %one : i64
      arc.yield %next
    }
    arc.return %r : i64
  }
}
"#;
        let module = parse_module(source).expect("loop parses");
        let func = module.functions.values().next().unwrap();
        let entry = &func.blocks[0];
        let loop_op = &entry.ops[0];
        assert!(matches!(loop_op.kind, OperationKind::Loop { .. }));
        if let OperationKind::Loop { iter_args } = &loop_op.kind {
            assert_eq!(iter_args.len(), 1);
            assert_eq!(iter_args[0].as_str(), "i");
        }
        assert_eq!(loop_op.regions.len(), 1);
        assert_eq!(loop_op.regions[0].blocks.len(), 1);
        // body has const + add + yield = 3 ops
        assert_eq!(loop_op.regions[0].blocks[0].ops.len(), 3);
    }

    #[test]
    fn parse_yield_standalone() {
        let source = r#"
arc.module @m {
  arc.func @f() {
  ^entry:
    %x = arc.const 5 : i64
    arc.yield %x
  }
}
"#;
        let module = parse_module(source).expect("yield parses");
        let func = module.functions.values().next().unwrap();
        let entry = &func.blocks[0];
        let yield_op = &entry.ops[1];
        assert!(matches!(yield_op.kind, OperationKind::Yield));
        assert_eq!(yield_op.operands.len(), 1);
        assert_eq!(yield_op.operands[0].as_str(), "x");
    }

    #[test]
    fn parse_if_roundtrip() {
        // Build an if op programmatically, print it, parse it back
        use arc_ir::Region;

        let mut module = Module::new(Symbol::new("m"));
        let mut func = Function::new(
            Symbol::new("f"),
            vec![],
            vec![arc_ir::Argument {
                name: ValueId::new("cond"),
                ty: Type::new("i1"),
                location: Location::new(0, 0),
            }],
            Some(Type::new("i64")),
            Location::new(0, 0),
        );
        let mut entry = arc_ir::Block::new(Some("entry".into()), Location::new(0, 0));

        // Create then region
        let mut then_block = arc_ir::Block::new(Some("then".into()), Location::new(0, 0));
        then_block.add_op(Operation {
            results: vec![ValueId::new("a")],
            kind: OperationKind::ConstI64(10),
            operands: vec![],
            result_types: vec![Type::new("i64")],
            effects: vec![],
            location: Location::new(0, 0),
            regions: vec![],
        });
        then_block.add_op(Operation {
            results: vec![],
            kind: OperationKind::Yield,
            operands: vec![ValueId::new("a")],
            result_types: vec![],
            effects: vec![],
            location: Location::new(0, 0),
            regions: vec![],
        });
        let then_region = Region::with_blocks(vec![then_block]);

        // Create else region
        let mut else_block = arc_ir::Block::new(Some("else_b".into()), Location::new(0, 0));
        else_block.add_op(Operation {
            results: vec![ValueId::new("b")],
            kind: OperationKind::ConstI64(20),
            operands: vec![],
            result_types: vec![Type::new("i64")],
            effects: vec![],
            location: Location::new(0, 0),
            regions: vec![],
        });
        else_block.add_op(Operation {
            results: vec![],
            kind: OperationKind::Yield,
            operands: vec![ValueId::new("b")],
            result_types: vec![],
            effects: vec![],
            location: Location::new(0, 0),
            regions: vec![],
        });
        let else_region = Region::with_blocks(vec![else_block]);

        entry.add_op(Operation {
            results: vec![ValueId::new("r")],
            kind: OperationKind::If,
            operands: vec![ValueId::new("cond")],
            result_types: vec![],
            effects: vec![],
            location: Location::new(0, 0),
            regions: vec![then_region, else_region],
        });
        entry.add_op(Operation {
            results: vec![],
            kind: OperationKind::Return,
            operands: vec![ValueId::new("r")],
            result_types: vec![Type::new("i64")],
            effects: vec![],
            location: Location::new(0, 0),
            regions: vec![],
        });
        func.add_block(entry);
        module.add_function(func).unwrap();

        let printed = print_module(&module);
        let reparsed = parse_module(&printed).expect("round-trip parse should succeed");
        let rf = reparsed.functions.values().next().unwrap();
        let if_op = &rf.blocks[0].ops[0];
        assert!(matches!(if_op.kind, OperationKind::If));
        assert_eq!(if_op.regions.len(), 2);
        assert_eq!(if_op.regions[0].blocks[0].ops.len(), 2);
        assert_eq!(if_op.regions[1].blocks[0].ops.len(), 2);
    }

    #[test]
    fn parse_load_elem_operation() {
        let source = r#"
arc.module @m {
  arc.func @access forall (%n: index) (%xs: !arc.slice<i32, %n>, %idx: index, %pf: !arc.proof<%idx < %n>) -> i32 {
  ^entry:
    %value = arc.load_elem %xs[%idx] requires %pf : (!arc.slice<i32, %n>, index) -> i32
    arc.return %value : i32
  }
}
"#;
        let module = parse_module(source).expect("module parses");
        let func = module.functions.values().next().expect("function present");
        assert_eq!(func.blocks[0].ops.len(), 2);
        let load = &func.blocks[0].ops[0];
        assert!(matches!(load.kind, OperationKind::LoadElem));
        assert_eq!(load.operands.len(), 3);
        assert_eq!(load.results.len(), 1);
        assert_eq!(load.result_types[0].as_str(), "i32");
    }
}
