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
}

impl Module {
    pub fn new(name: Symbol) -> Self {
        Self {
            name,
            functions: IndexMap::new(),
        }
    }

    pub fn add_function(&mut self, func: Function) -> Result<(), ModuleError> {
        if self.functions.contains_key(&func.name) {
            return Err(ModuleError::DuplicateSymbol(func.name.clone()));
        }
        self.functions.insert(func.name.clone(), func);
        Ok(())
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    pub results: Vec<ValueId>,
    pub kind: OperationKind,
    pub operands: Vec<ValueId>,
    pub result_types: Vec<Type>,
    pub location: Location,
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
    Branch {
        target: BlockTarget,
    },
    CondBranch {
        true_target: BlockTarget,
        false_target: BlockTarget,
    },
    Return,
    Unknown(SmolStr),
}

#[derive(thiserror::Error, Debug)]
pub enum ModuleError {
    #[error("duplicate symbol {0}")]
    DuplicateSymbol(Symbol),
}
