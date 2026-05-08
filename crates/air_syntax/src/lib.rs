mod parser;

pub use parser::{parse_module, ParseError};

#[cfg(test)]
mod tests {
    use super::*;
    use air_ir::OperationKind;

    #[test]
    fn parse_valid_spec_example() {
        let source = r#"
air.module @m {
  air.func @add(%a: i64, %b: i64) -> i64 {
  ^entry:
    %c = air.const 1 : i64
    %d = air.add %a, %b : i64
    air.return %d : i64
  }
}
"#;
        let module = parse_module(source).expect("module parses");
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn reject_missing_type() {
        let source = r#"
air.module @bad {
  air.func @add(%a: i64, %b: i64) {
  ^entry:
    %c = air.add %a, %b
    air.return %c
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
air.module @m {
  air.func @branch(%cond: i1, %x: i64) -> i64 {
  ^entry(%arg0: i64):
    air.cond_br %cond, ^then(%arg0), ^else(%arg0)
  ^then(%y: i64):
    %one = air.const 1 : i64
    %sum = air.add %y, %one : i64
    air.br ^exit(%sum)
  ^else(%z: i64):
    air.br ^exit(%z)
  ^exit(%result: i64):
    air.return %result : i64
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
}
