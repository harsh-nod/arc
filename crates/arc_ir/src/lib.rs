use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol(SmolStr);

impl Symbol {
    pub fn new(name: impl Into<SmolStr>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValueId(SmolStr);

impl ValueId {
    pub fn new(name: impl Into<SmolStr>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ValueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Type {
    repr: SmolStr,
}

impl Type {
    pub fn new(repr: impl Into<SmolStr>) -> Self {
        Self { repr: repr.into() }
    }

    pub fn as_str(&self) -> &str {
        &self.repr
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.repr)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub offset: usize,
    pub length: usize,
}

impl Location {
    pub fn new(offset: usize, length: usize) -> Self {
        Self { offset, length }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    pub name: Symbol,
    pub functions: IndexMap<Symbol, Function>,
    pub capabilities: IndexMap<Symbol, Capability>,
}

impl Module {
    pub fn new(name: Symbol) -> Self {
        Self {
            name,
            functions: IndexMap::new(),
            capabilities: IndexMap::new(),
        }
    }

    pub fn add_function(&mut self, func: Function) -> Result<(), ModuleError> {
        if self.functions.contains_key(&func.name) {
            return Err(ModuleError::DuplicateSymbol(func.name.clone()));
        }
        self.functions.insert(func.name.clone(), func);
        Ok(())
    }

    pub fn add_capability(&mut self, cap: Capability) -> Result<(), ModuleError> {
        if self.capabilities.contains_key(&cap.name) {
            return Err(ModuleError::DuplicateSymbol(cap.name.clone()));
        }
        self.capabilities.insert(cap.name.clone(), cap);
        Ok(())
    }
}

/// A capability declaration describing an external action the program can invoke.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub name: Symbol,
    pub inputs: Vec<Argument>,
    pub outputs: Vec<Argument>,
    pub effects: Vec<String>,
    pub failures: Vec<String>,
    pub location: Location,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexParam {
    pub name: ValueId,
    pub location: Location,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub name: Symbol,
    pub index_params: Vec<IndexParam>,
    pub params: Vec<Argument>,
    pub result: Option<Type>,
    pub blocks: Vec<Block>,
    pub location: Location,
}

impl Function {
    pub fn new(
        name: Symbol,
        index_params: Vec<IndexParam>,
        params: Vec<Argument>,
        result: Option<Type>,
        location: Location,
    ) -> Self {
        Self {
            name,
            index_params,
            params,
            result,
            blocks: Vec::new(),
            location,
        }
    }

    pub fn add_block(&mut self, block: Block) {
        self.blocks.push(block);
    }

    pub fn block_index_by_label(&self, label: &str) -> Option<usize> {
        self.blocks.iter().position(|block| {
            block
                .label
                .as_ref()
                .map(|existing| existing.as_str() == label)
                .unwrap_or(false)
        })
    }

    pub fn block_by_label(&self, label: &str) -> Option<&Block> {
        self.block_index_by_label(label)
            .and_then(|idx| self.blocks.get(idx))
    }

    pub fn entry_block(&self) -> Option<&Block> {
        self.blocks.first()
    }

    pub fn add_index_param(&mut self, param: IndexParam) {
        self.index_params.push(param);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Argument {
    pub name: ValueId,
    pub ty: Type,
    pub location: Location,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub label: Option<SmolStr>,
    pub args: Vec<Argument>,
    pub ops: Vec<Operation>,
    pub location: Location,
}

impl Block {
    pub fn new(label: Option<SmolStr>, location: Location) -> Self {
        Self {
            label,
            args: Vec::new(),
            ops: Vec::new(),
            location,
        }
    }

    pub fn label(&self) -> Option<&SmolStr> {
        self.label.as_ref()
    }

    pub fn add_arg(&mut self, arg: Argument) {
        self.args.push(arg);
    }

    pub fn add_op(&mut self, op: Operation) {
        self.ops.push(op);
    }

    pub fn terminator(&self) -> Option<&Operation> {
        self.ops.last()
    }
}

/// A region is a nested scope containing blocks, used by structured ops like `if` and `loop`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub blocks: Vec<Block>,
}

impl Region {
    pub fn new() -> Self {
        Self { blocks: Vec::new() }
    }

    pub fn with_blocks(blocks: Vec<Block>) -> Self {
        Self { blocks }
    }

    pub fn entry_block(&self) -> Option<&Block> {
        self.blocks.first()
    }

    pub fn add_block(&mut self, block: Block) {
        self.blocks.push(block);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    pub results: Vec<ValueId>,
    pub kind: OperationKind,
    pub operands: Vec<ValueId>,
    pub result_types: Vec<Type>,
    pub effects: Vec<String>,
    pub location: Location,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<Region>,
}

impl Operation {
    /// Create an operation with no regions (the common case).
    pub fn simple(
        results: Vec<ValueId>,
        kind: OperationKind,
        operands: Vec<ValueId>,
        result_types: Vec<Type>,
        effects: Vec<String>,
        location: Location,
    ) -> Self {
        Self {
            results,
            kind,
            operands,
            result_types,
            effects,
            location,
            regions: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockTarget {
    pub label: SmolStr,
    pub arguments: Vec<ValueId>,
}

impl BlockTarget {
    pub fn new(label: SmolStr, arguments: Vec<ValueId>) -> Self {
        Self { label, arguments }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IcmpPredicate {
    Eq,
    Ne,
    Slt,
    Sle,
    Sgt,
    Sge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationKind {
    ConstI64(i64),
    Add,
    Sub,
    Mul,
    Div,
    ICmp {
        predicate: IcmpPredicate,
    },
    Alloc,
    Load,
    Store,
    LoadElem,
    Assume,
    Assert,
    Prove,
    Refine,
    Branch {
        target: BlockTarget,
    },
    CondBranch {
        true_target: BlockTarget,
        false_target: BlockTarget,
    },
    Call {
        callee: Symbol,
    },
    /// Request human approval, producing an authority token.
    RequireApproval,
    /// Invoke an external capability (e.g. email.send, file.read).
    Invoke {
        capability: Symbol,
    },
    Return,
    /// Structured if: condition in `operands[0]`, `regions[0]` is then, `regions[1]` is else.
    If,
    /// Structured loop: `regions[0]` is body, iter_args passed via Yield.
    Loop {
        iter_args: Vec<ValueId>,
    },
    /// Yield values from a region (terminates loop body or if body).
    Yield,
    /// Spawn an async task. `operands[0]` is the task argument payload.
    /// Result is a task handle (i64 task id).
    Spawn {
        callee: Symbol,
    },
    /// Await a spawned task. `operands[0]` is the task handle. Result is the task's return value.
    Await,
    /// Checkpoint: suspend the current task for later resumption.
    /// Produces a continuation token for the checkpoint label.
    Checkpoint {
        label: SmolStr,
    },
    Unknown(SmolStr),
}

#[derive(thiserror::Error, Debug)]
pub enum ModuleError {
    #[error("duplicate symbol {0}")]
    DuplicateSymbol(Symbol),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc() -> Location {
        Location::new(0, 0)
    }

    // --- Symbol ---

    #[test]
    fn symbol_display_prefixed_with_at() {
        let s = Symbol::new("main");
        assert_eq!(format!("{}", s), "@main");
    }

    #[test]
    fn symbol_as_str() {
        let s = Symbol::new("foo");
        assert_eq!(s.as_str(), "foo");
    }

    #[test]
    fn symbol_equality() {
        assert_eq!(Symbol::new("x"), Symbol::new("x"));
        assert_ne!(Symbol::new("x"), Symbol::new("y"));
    }

    // --- ValueId ---

    #[test]
    fn value_id_display_prefixed_with_percent() {
        let v = ValueId::new("r0");
        assert_eq!(format!("{}", v), "%r0");
    }

    #[test]
    fn value_id_equality_and_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(ValueId::new("a"));
        set.insert(ValueId::new("a"));
        assert_eq!(set.len(), 1);
    }

    // --- Type ---

    #[test]
    fn type_display() {
        let t = Type::new("i64");
        assert_eq!(format!("{}", t), "i64");
    }

    #[test]
    fn type_as_str() {
        let t = Type::new("!arc.proof<true>");
        assert_eq!(t.as_str(), "!arc.proof<true>");
    }

    // --- Location ---

    #[test]
    fn location_fields() {
        let l = Location::new(10, 5);
        assert_eq!(l.offset, 10);
        assert_eq!(l.length, 5);
    }

    // --- Module ---

    #[test]
    fn module_new_empty() {
        let m = Module::new(Symbol::new("test"));
        assert_eq!(m.name.as_str(), "test");
        assert!(m.functions.is_empty());
        assert!(m.capabilities.is_empty());
    }

    #[test]
    fn module_add_function() {
        let mut m = Module::new(Symbol::new("m"));
        let f = Function::new(Symbol::new("main"), vec![], vec![], None, loc());
        assert!(m.add_function(f).is_ok());
        assert_eq!(m.functions.len(), 1);
    }

    #[test]
    fn module_duplicate_function_rejected() {
        let mut m = Module::new(Symbol::new("m"));
        let f1 = Function::new(Symbol::new("main"), vec![], vec![], None, loc());
        let f2 = Function::new(Symbol::new("main"), vec![], vec![], None, loc());
        assert!(m.add_function(f1).is_ok());
        let err = m.add_function(f2).unwrap_err();
        assert!(err.to_string().contains("main"));
    }

    #[test]
    fn module_add_capability() {
        let mut m = Module::new(Symbol::new("m"));
        let cap = Capability {
            name: Symbol::new("email.send"),
            inputs: vec![],
            outputs: vec![],
            effects: vec!["io".to_string()],
            failures: vec![],
            location: loc(),
        };
        assert!(m.add_capability(cap).is_ok());
        assert_eq!(m.capabilities.len(), 1);
    }

    #[test]
    fn module_duplicate_capability_rejected() {
        let mut m = Module::new(Symbol::new("m"));
        let cap1 = Capability {
            name: Symbol::new("fs.read"),
            inputs: vec![],
            outputs: vec![],
            effects: vec![],
            failures: vec![],
            location: loc(),
        };
        let cap2 = cap1.clone();
        assert!(m.add_capability(cap1).is_ok());
        assert!(m.add_capability(cap2).is_err());
    }

    // --- Function ---

    #[test]
    fn function_add_block_and_entry() {
        let mut f = Function::new(Symbol::new("f"), vec![], vec![], None, loc());
        assert!(f.entry_block().is_none());
        f.add_block(Block::new(Some("entry".into()), loc()));
        assert!(f.entry_block().is_some());
        assert_eq!(f.blocks.len(), 1);
    }

    #[test]
    fn function_block_by_label() {
        let mut f = Function::new(Symbol::new("f"), vec![], vec![], None, loc());
        f.add_block(Block::new(Some("entry".into()), loc()));
        f.add_block(Block::new(Some("loop".into()), loc()));
        f.add_block(Block::new(Some("exit".into()), loc()));
        assert_eq!(f.block_index_by_label("loop"), Some(1));
        assert!(f.block_by_label("exit").is_some());
        assert!(f.block_by_label("nonexistent").is_none());
    }

    #[test]
    fn function_add_index_param() {
        let mut f = Function::new(Symbol::new("f"), vec![], vec![], None, loc());
        f.add_index_param(IndexParam {
            name: ValueId::new("n"),
            location: loc(),
        });
        assert_eq!(f.index_params.len(), 1);
    }

    // --- Block ---

    #[test]
    fn block_label() {
        let b = Block::new(Some("entry".into()), loc());
        assert_eq!(b.label().unwrap().as_str(), "entry");

        let b2 = Block::new(None, loc());
        assert!(b2.label().is_none());
    }

    #[test]
    fn block_add_arg_and_op() {
        let mut b = Block::new(Some("bb".into()), loc());
        b.add_arg(Argument {
            name: ValueId::new("x"),
            ty: Type::new("i64"),
            location: loc(),
        });
        assert_eq!(b.args.len(), 1);

        b.add_op(Operation::simple(
            vec![ValueId::new("c")],
            OperationKind::ConstI64(42),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        assert_eq!(b.ops.len(), 1);
    }

    #[test]
    fn block_terminator_is_last_op() {
        let mut b = Block::new(Some("bb".into()), loc());
        assert!(b.terminator().is_none());
        b.add_op(Operation::simple(
            vec![ValueId::new("c")],
            OperationKind::ConstI64(1),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        b.add_op(Operation::simple(
            vec![],
            OperationKind::Return,
            vec![ValueId::new("c")],
            vec![],
            vec![],
            loc(),
        ));
        assert!(matches!(
            b.terminator().unwrap().kind,
            OperationKind::Return
        ));
    }

    // --- Region ---

    #[test]
    fn region_new_empty() {
        let r = Region::new();
        assert!(r.blocks.is_empty());
        assert!(r.entry_block().is_none());
    }

    #[test]
    fn region_with_blocks() {
        let b = Block::new(Some("body".into()), loc());
        let r = Region::with_blocks(vec![b]);
        assert_eq!(r.blocks.len(), 1);
        assert_eq!(r.entry_block().unwrap().label().unwrap().as_str(), "body");
    }

    #[test]
    fn region_add_block() {
        let mut r = Region::new();
        r.add_block(Block::new(Some("a".into()), loc()));
        r.add_block(Block::new(Some("b".into()), loc()));
        assert_eq!(r.blocks.len(), 2);
    }

    // --- Operation ---

    #[test]
    fn operation_simple_has_no_regions() {
        let op = Operation::simple(
            vec![ValueId::new("x")],
            OperationKind::ConstI64(0),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        );
        assert!(op.regions.is_empty());
    }

    #[test]
    fn operation_with_regions() {
        let then_block = Block::new(Some("then".into()), loc());
        let else_block = Block::new(Some("else".into()), loc());
        let op = Operation {
            results: vec![ValueId::new("r")],
            kind: OperationKind::If,
            operands: vec![ValueId::new("cond")],
            result_types: vec![Type::new("i64")],
            effects: vec![],
            location: loc(),
            regions: vec![
                Region::with_blocks(vec![then_block]),
                Region::with_blocks(vec![else_block]),
            ],
        };
        assert_eq!(op.regions.len(), 2);
    }

    // --- BlockTarget ---

    #[test]
    fn block_target_new() {
        let bt = BlockTarget::new("loop".into(), vec![ValueId::new("i")]);
        assert_eq!(bt.label.as_str(), "loop");
        assert_eq!(bt.arguments.len(), 1);
    }

    // --- OperationKind variants ---

    #[test]
    fn operation_kind_variants_exist() {
        // Smoke test that all variants can be constructed
        let _ = OperationKind::ConstI64(42);
        let _ = OperationKind::Add;
        let _ = OperationKind::Sub;
        let _ = OperationKind::Mul;
        let _ = OperationKind::Div;
        let _ = OperationKind::ICmp {
            predicate: IcmpPredicate::Eq,
        };
        let _ = OperationKind::Alloc;
        let _ = OperationKind::Load;
        let _ = OperationKind::Store;
        let _ = OperationKind::LoadElem;
        let _ = OperationKind::Assume;
        let _ = OperationKind::Assert;
        let _ = OperationKind::Prove;
        let _ = OperationKind::Refine;
        let _ = OperationKind::Branch {
            target: BlockTarget::new("b".into(), vec![]),
        };
        let _ = OperationKind::CondBranch {
            true_target: BlockTarget::new("t".into(), vec![]),
            false_target: BlockTarget::new("f".into(), vec![]),
        };
        let _ = OperationKind::Call {
            callee: Symbol::new("f"),
        };
        let _ = OperationKind::RequireApproval;
        let _ = OperationKind::Invoke {
            capability: Symbol::new("c"),
        };
        let _ = OperationKind::Return;
        let _ = OperationKind::If;
        let _ = OperationKind::Loop { iter_args: vec![] };
        let _ = OperationKind::Yield;
        let _ = OperationKind::Spawn {
            callee: Symbol::new("worker"),
        };
        let _ = OperationKind::Await;
        let _ = OperationKind::Checkpoint {
            label: "cp1".into(),
        };
        let _ = OperationKind::Unknown("custom.op".into());
    }

    // --- Serde round-trip ---

    #[test]
    fn module_serde_roundtrip() {
        let mut m = Module::new(Symbol::new("test"));
        let mut f = Function::new(
            Symbol::new("main"),
            vec![],
            vec![Argument {
                name: ValueId::new("x"),
                ty: Type::new("i64"),
                location: loc(),
            }],
            Some(Type::new("i64")),
            loc(),
        );
        let mut block = Block::new(Some("entry".into()), loc());
        block.add_op(Operation::simple(
            vec![ValueId::new("c")],
            OperationKind::ConstI64(42),
            vec![],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        block.add_op(Operation::simple(
            vec![],
            OperationKind::Return,
            vec![ValueId::new("c")],
            vec![Type::new("i64")],
            vec![],
            loc(),
        ));
        f.add_block(block);
        m.add_function(f).unwrap();

        let json = serde_json::to_string(&m).unwrap();
        let restored: Module = serde_json::from_str(&json).unwrap();
        assert_eq!(m, restored);
    }

    #[test]
    fn operation_regions_serde_default() {
        // An operation serialized without regions should deserialize with empty regions
        let op = Operation::simple(vec![], OperationKind::Return, vec![], vec![], vec![], loc());
        let json = serde_json::to_string(&op).unwrap();
        assert!(!json.contains("regions"), "empty regions should be skipped");
        let restored: Operation = serde_json::from_str(&json).unwrap();
        assert!(restored.regions.is_empty());
    }

    #[test]
    fn operation_with_regions_serde_roundtrip() {
        let op = Operation {
            results: vec![ValueId::new("r")],
            kind: OperationKind::If,
            operands: vec![ValueId::new("c")],
            result_types: vec![Type::new("i64")],
            effects: vec![],
            location: loc(),
            regions: vec![
                Region::with_blocks(vec![Block::new(Some("then".into()), loc())]),
                Region::with_blocks(vec![Block::new(Some("else".into()), loc())]),
            ],
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains("regions"));
        let restored: Operation = serde_json::from_str(&json).unwrap();
        assert_eq!(op, restored);
    }

    #[test]
    fn icmp_predicate_serde_roundtrip() {
        for pred in [
            IcmpPredicate::Eq,
            IcmpPredicate::Ne,
            IcmpPredicate::Slt,
            IcmpPredicate::Sle,
            IcmpPredicate::Sgt,
            IcmpPredicate::Sge,
        ] {
            let json = serde_json::to_string(&pred).unwrap();
            let restored: IcmpPredicate = serde_json::from_str(&json).unwrap();
            assert_eq!(pred, restored);
        }
    }

    // --- Edge cases ---

    #[test]
    fn empty_module_serde() {
        let m = Module::new(Symbol::new("empty"));
        let json = serde_json::to_string(&m).unwrap();
        let restored: Module = serde_json::from_str(&json).unwrap();
        assert_eq!(m, restored);
    }

    #[test]
    fn function_with_no_blocks() {
        let f = Function::new(Symbol::new("f"), vec![], vec![], None, loc());
        assert!(f.entry_block().is_none());
        assert!(f.block_by_label("anything").is_none());
        assert_eq!(f.block_index_by_label("anything"), None);
    }

    #[test]
    fn block_with_no_label() {
        let b = Block::new(None, loc());
        assert!(b.label().is_none());
    }

    #[test]
    fn symbol_from_string_and_str() {
        let s1 = Symbol::new("abc");
        let s2 = Symbol::new(String::from("abc"));
        assert_eq!(s1, s2);
    }

    #[test]
    fn module_error_display() {
        let err = ModuleError::DuplicateSymbol(Symbol::new("foo"));
        assert_eq!(err.to_string(), "duplicate symbol @foo");
    }
}
