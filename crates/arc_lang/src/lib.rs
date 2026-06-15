//! MiniAIR: a small imperative source language that compiles to AIR.
//!
//! Syntax:
//! ```text
//! fn add(a: i64, b: i64) -> i64 {
//!     let c = a + b;
//!     return c;
//! }
//!
//! fn max(a: i64, b: i64) -> i64 {
//!     if a > b {
//!         return a;
//!     } else {
//!         return b;
//!     }
//! }
//! ```

use arc_ir::*;
use smol_str::SmolStr;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    I64,
    I32,
    Bool,
    Index,
    Void,
}

impl Ty {
    fn to_air_type(&self) -> Type {
        match self {
            Ty::I64 => Type::new("i64"),
            Ty::I32 => Type::new("i32"),
            Ty::Bool => Type::new("i1"),
            Ty::Index => Type::new("index"),
            Ty::Void => Type::new("void"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    IntLit(i64),
    Var(String),
    BinOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Cmp {
        op: CmpOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        ty: Option<Ty>,
        value: Expr,
    },
    Return(Option<Expr>),
    If {
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    Assign {
        name: String,
        value: Expr,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Ty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<Param>,
    pub ret_ty: Ty,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub functions: Vec<FuncDef>,
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Fn,
    Let,
    Return,
    If,
    Else,
    Arrow, // ->
    Colon,
    Semicolon,
    Comma,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Plus,
    Minus,
    Star,
    Slash,
    EqEq,
    BangEq,
    Lt,
    Le,
    Gt,
    Ge,
    Eq, // =
    Ident(String),
    IntLit(i64),
    Eof,
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        self.pos += 1;
        c
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else if c == '/' && self.chars.get(self.pos + 1) == Some(&'/') {
                // Line comment.
                while let Some(c) = self.advance() {
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn tokenize(&mut self) -> Result<Vec<Token>, LangError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => {
                    tokens.push(Token::Eof);
                    return Ok(tokens);
                }
                Some(c) => {
                    let tok = match c {
                        '(' => {
                            self.advance();
                            Token::LParen
                        }
                        ')' => {
                            self.advance();
                            Token::RParen
                        }
                        '{' => {
                            self.advance();
                            Token::LBrace
                        }
                        '}' => {
                            self.advance();
                            Token::RBrace
                        }
                        ':' => {
                            self.advance();
                            Token::Colon
                        }
                        ';' => {
                            self.advance();
                            Token::Semicolon
                        }
                        ',' => {
                            self.advance();
                            Token::Comma
                        }
                        '+' => {
                            self.advance();
                            Token::Plus
                        }
                        '*' => {
                            self.advance();
                            Token::Star
                        }
                        '/' => {
                            self.advance();
                            Token::Slash
                        }
                        '-' => {
                            self.advance();
                            if self.peek() == Some('>') {
                                self.advance();
                                Token::Arrow
                            } else {
                                Token::Minus
                            }
                        }
                        '=' => {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                Token::EqEq
                            } else {
                                Token::Eq
                            }
                        }
                        '!' => {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                Token::BangEq
                            } else {
                                return Err(LangError::LexError(format!(
                                    "unexpected '!' at position {}",
                                    self.pos
                                )));
                            }
                        }
                        '<' => {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                Token::Le
                            } else {
                                Token::Lt
                            }
                        }
                        '>' => {
                            self.advance();
                            if self.peek() == Some('=') {
                                self.advance();
                                Token::Ge
                            } else {
                                Token::Gt
                            }
                        }
                        _ if c.is_ascii_digit() => {
                            let mut num = String::new();
                            while let Some(d) = self.peek() {
                                if d.is_ascii_digit() {
                                    num.push(d);
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                            Token::IntLit(
                                num.parse()
                                    .map_err(|e| LangError::LexError(format!("{}", e)))?,
                            )
                        }
                        _ if c.is_ascii_alphabetic() || c == '_' => {
                            let mut ident = String::new();
                            while let Some(ch) = self.peek() {
                                if ch.is_ascii_alphanumeric() || ch == '_' {
                                    ident.push(ch);
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                            match ident.as_str() {
                                "fn" => Token::Fn,
                                "let" => Token::Let,
                                "return" => Token::Return,
                                "if" => Token::If,
                                "else" => Token::Else,
                                _ => Token::Ident(ident),
                            }
                        }
                        _ => {
                            return Err(LangError::LexError(format!(
                                "unexpected character '{}' at position {}",
                                c, self.pos
                            )));
                        }
                    };
                    tokens.push(tok);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), LangError> {
        let tok = self.advance();
        if &tok != expected {
            Err(LangError::ParseError(format!(
                "expected {:?}, got {:?}",
                expected, tok
            )))
        } else {
            Ok(())
        }
    }

    fn parse_program(&mut self) -> Result<Program, LangError> {
        let mut functions = Vec::new();
        while self.peek() != &Token::Eof {
            functions.push(self.parse_func()?);
        }
        Ok(Program { functions })
    }

    fn parse_func(&mut self) -> Result<FuncDef, LangError> {
        self.expect(&Token::Fn)?;
        let name = match self.advance() {
            Token::Ident(n) => n,
            t => {
                return Err(LangError::ParseError(format!(
                    "expected function name, got {:?}",
                    t
                )))
            }
        };
        self.expect(&Token::LParen)?;
        let params = self.parse_params()?;
        self.expect(&Token::RParen)?;

        let ret_ty = if self.peek() == &Token::Arrow {
            self.advance();
            self.parse_type()?
        } else {
            Ty::Void
        };

        self.expect(&Token::LBrace)?;
        let body = self.parse_block()?;
        self.expect(&Token::RBrace)?;

        Ok(FuncDef {
            name,
            params,
            ret_ty,
            body,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, LangError> {
        let mut params = Vec::new();
        if self.peek() == &Token::RParen {
            return Ok(params);
        }
        loop {
            let name = match self.advance() {
                Token::Ident(n) => n,
                t => {
                    return Err(LangError::ParseError(format!(
                        "expected parameter name, got {:?}",
                        t
                    )))
                }
            };
            self.expect(&Token::Colon)?;
            let ty = self.parse_type()?;
            params.push(Param { name, ty });
            if self.peek() != &Token::Comma {
                break;
            }
            self.advance(); // consume comma
        }
        Ok(params)
    }

    fn parse_type(&mut self) -> Result<Ty, LangError> {
        match self.advance() {
            Token::Ident(t) => match t.as_str() {
                "i64" => Ok(Ty::I64),
                "i32" => Ok(Ty::I32),
                "bool" => Ok(Ty::Bool),
                "index" => Ok(Ty::Index),
                "void" => Ok(Ty::Void),
                _ => Err(LangError::ParseError(format!("unknown type '{}'", t))),
            },
            t => Err(LangError::ParseError(format!("expected type, got {:?}", t))),
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, LangError> {
        let mut stmts = Vec::new();
        while self.peek() != &Token::RBrace && self.peek() != &Token::Eof {
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, LangError> {
        match self.peek().clone() {
            Token::Let => {
                self.advance();
                let name = match self.advance() {
                    Token::Ident(n) => n,
                    t => {
                        return Err(LangError::ParseError(format!(
                            "expected variable name, got {:?}",
                            t
                        )))
                    }
                };
                let ty = if self.peek() == &Token::Colon {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.expect(&Token::Eq)?;
                let value = self.parse_expr()?;
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::Let { name, ty, value })
            }
            Token::Return => {
                self.advance();
                if self.peek() == &Token::Semicolon {
                    self.advance();
                    Ok(Stmt::Return(None))
                } else {
                    let expr = self.parse_expr()?;
                    self.expect(&Token::Semicolon)?;
                    Ok(Stmt::Return(Some(expr)))
                }
            }
            Token::If => {
                self.advance();
                let condition = self.parse_expr()?;
                self.expect(&Token::LBrace)?;
                let then_body = self.parse_block()?;
                self.expect(&Token::RBrace)?;
                let else_body = if self.peek() == &Token::Else {
                    self.advance();
                    self.expect(&Token::LBrace)?;
                    let body = self.parse_block()?;
                    self.expect(&Token::RBrace)?;
                    body
                } else {
                    Vec::new()
                };
                Ok(Stmt::If {
                    condition,
                    then_body,
                    else_body,
                })
            }
            Token::Ident(_) => {
                // Could be assignment: name = expr;
                let name = match self.advance() {
                    Token::Ident(n) => n,
                    _ => unreachable!(),
                };
                if self.peek() == &Token::Eq {
                    self.advance();
                    let value = self.parse_expr()?;
                    self.expect(&Token::Semicolon)?;
                    Ok(Stmt::Assign { name, value })
                } else {
                    // Expression statement (function call).
                    // Put back by parsing as call.
                    Err(LangError::ParseError(format!(
                        "expected '=' after identifier '{}', got {:?}",
                        name,
                        self.peek()
                    )))
                }
            }
            t => Err(LangError::ParseError(format!(
                "unexpected token {:?} at start of statement",
                t
            ))),
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, LangError> {
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, LangError> {
        let mut lhs = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                Token::EqEq => CmpOp::Eq,
                Token::BangEq => CmpOp::Ne,
                Token::Lt => CmpOp::Lt,
                Token::Le => CmpOp::Le,
                Token::Gt => CmpOp::Gt,
                Token::Ge => CmpOp::Ge,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_additive()?;
            lhs = Expr::Cmp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_additive(&mut self) -> Result<Expr, LangError> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_multiplicative()?;
            lhs = Expr::BinOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, LangError> {
        let mut lhs = self.parse_primary()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_primary()?;
            lhs = Expr::BinOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_primary(&mut self) -> Result<Expr, LangError> {
        match self.peek().clone() {
            Token::IntLit(n) => {
                self.advance();
                Ok(Expr::IntLit(n))
            }
            Token::Ident(name) => {
                self.advance();
                if self.peek() == &Token::LParen {
                    // Function call.
                    self.advance();
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        loop {
                            args.push(self.parse_expr()?);
                            if self.peek() != &Token::Comma {
                                break;
                            }
                            self.advance();
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::Call { name, args })
                } else {
                    Ok(Expr::Var(name))
                }
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            t => Err(LangError::ParseError(format!(
                "unexpected token {:?} in expression",
                t
            ))),
        }
    }
}

/// Parse a MiniAIR source string into a Program AST.
pub fn parse(source: &str) -> Result<Program, LangError> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse_program()
}

// ---------------------------------------------------------------------------
// Compiler: AST -> AIR Module
// ---------------------------------------------------------------------------

struct Compiler {
    next_id: u32,
    next_block: u32,
}

impl Compiler {
    fn new() -> Self {
        Self {
            next_id: 0,
            next_block: 0,
        }
    }

    fn fresh_var(&mut self) -> ValueId {
        let id = self.next_id;
        self.next_id += 1;
        ValueId::new(format!("v{}", id))
    }

    fn fresh_label(&mut self) -> SmolStr {
        let id = self.next_block;
        self.next_block += 1;
        SmolStr::new(format!("bb{}", id))
    }

    fn loc() -> Location {
        Location::new(0, 0)
    }

    fn compile_program(&mut self, program: &Program) -> Result<Module, LangError> {
        let mut module = Module::new(Symbol::new("main"));
        for func_def in &program.functions {
            let func = self.compile_func(func_def)?;
            module
                .add_function(func)
                .map_err(|e| LangError::CompileError(e.to_string()))?;
        }
        Ok(module)
    }

    fn compile_func(&mut self, func_def: &FuncDef) -> Result<Function, LangError> {
        self.next_id = 0;
        self.next_block = 0;

        let params: Vec<Argument> = func_def
            .params
            .iter()
            .map(|p| Argument {
                name: ValueId::new(&p.name),
                ty: p.ty.to_air_type(),
                location: Self::loc(),
            })
            .collect();

        let result = if func_def.ret_ty == Ty::Void {
            None
        } else {
            Some(func_def.ret_ty.to_air_type())
        };

        let mut func = Function::new(
            Symbol::new(&func_def.name),
            vec![],
            params,
            result,
            Self::loc(),
        );

        let entry_label = self.fresh_label();
        let mut entry_block = Block::new(Some(entry_label), Self::loc());

        // Map from variable names to AIR ValueIds.
        let mut env: HashMap<String, ValueId> = HashMap::new();
        for p in &func_def.params {
            env.insert(p.name.clone(), ValueId::new(&p.name));
        }

        // Collect extra blocks (from if/else) separately so entry is first.
        let mut extra_blocks = Vec::new();
        self.compile_stmts(
            &func_def.body,
            &mut entry_block,
            &mut env,
            &mut extra_blocks,
        )?;

        func.add_block(entry_block);
        for b in extra_blocks {
            func.add_block(b);
        }
        Ok(func)
    }

    fn compile_stmts(
        &mut self,
        stmts: &[Stmt],
        block: &mut Block,
        env: &mut HashMap<String, ValueId>,
        extra_blocks: &mut Vec<Block>,
    ) -> Result<(), LangError> {
        for stmt in stmts {
            match stmt {
                Stmt::Let { name, value, .. } => {
                    let val = self.compile_expr(value, block, env)?;
                    env.insert(name.clone(), val);
                }
                Stmt::Assign { name, value } => {
                    let val = self.compile_expr(value, block, env)?;
                    env.insert(name.clone(), val);
                }
                Stmt::Return(Some(expr)) => {
                    let val = self.compile_expr(expr, block, env)?;
                    block.add_op(Operation {
                        results: vec![],
                        kind: OperationKind::Return,
                        operands: vec![val],
                        result_types: vec![],
                        effects: vec![],
                        location: Self::loc(),
                        regions: vec![],
                    });
                }
                Stmt::Return(None) => {
                    block.add_op(Operation {
                        results: vec![],
                        kind: OperationKind::Return,
                        operands: vec![],
                        result_types: vec![],
                        effects: vec![],
                        location: Self::loc(),
                        regions: vec![],
                    });
                }
                Stmt::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    let cond_val = self.compile_expr(condition, block, env)?;

                    let then_label = self.fresh_label();
                    let else_label = self.fresh_label();

                    block.add_op(Operation {
                        results: vec![],
                        kind: OperationKind::CondBranch {
                            true_target: BlockTarget::new(then_label.clone(), vec![]),
                            false_target: BlockTarget::new(else_label.clone(), vec![]),
                        },
                        operands: vec![cond_val],
                        result_types: vec![],
                        effects: vec![],
                        location: Self::loc(),
                        regions: vec![],
                    });

                    // Then block.
                    let mut then_block = Block::new(Some(then_label), Self::loc());
                    let mut then_env = env.clone();
                    self.compile_stmts(then_body, &mut then_block, &mut then_env, extra_blocks)?;
                    extra_blocks.push(then_block);

                    // Else block.
                    let mut else_block = Block::new(Some(else_label), Self::loc());
                    let mut else_env = env.clone();
                    if else_body.is_empty() {
                        else_block.add_op(Operation {
                            results: vec![],
                            kind: OperationKind::Return,
                            operands: vec![],
                            result_types: vec![],
                            effects: vec![],
                            location: Self::loc(),
                            regions: vec![],
                        });
                    } else {
                        self.compile_stmts(
                            else_body,
                            &mut else_block,
                            &mut else_env,
                            extra_blocks,
                        )?;
                    }
                    extra_blocks.push(else_block);
                }
            }
        }
        Ok(())
    }

    fn compile_expr(
        &mut self,
        expr: &Expr,
        block: &mut Block,
        env: &HashMap<String, ValueId>,
    ) -> Result<ValueId, LangError> {
        match expr {
            Expr::IntLit(n) => {
                let result = self.fresh_var();
                block.add_op(Operation {
                    results: vec![result.clone()],
                    kind: OperationKind::ConstI64(*n),
                    operands: vec![],
                    result_types: vec![Type::new("i64")],
                    effects: vec![],
                    location: Self::loc(),
                    regions: vec![],
                });
                Ok(result)
            }
            Expr::Var(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| LangError::CompileError(format!("undefined variable '{}'", name))),
            Expr::BinOp { op, lhs, rhs } => {
                let lhs_val = self.compile_expr(lhs, block, env)?;
                let rhs_val = self.compile_expr(rhs, block, env)?;
                let result = self.fresh_var();
                let kind = match op {
                    BinOp::Add => OperationKind::Add,
                    BinOp::Sub => OperationKind::Sub,
                    BinOp::Mul => OperationKind::Mul,
                    BinOp::Div => OperationKind::Div,
                };
                block.add_op(Operation {
                    results: vec![result.clone()],
                    kind,
                    operands: vec![lhs_val, rhs_val],
                    result_types: vec![Type::new("i64")],
                    effects: vec![],
                    location: Self::loc(),
                    regions: vec![],
                });
                Ok(result)
            }
            Expr::Cmp { op, lhs, rhs } => {
                let lhs_val = self.compile_expr(lhs, block, env)?;
                let rhs_val = self.compile_expr(rhs, block, env)?;
                let result = self.fresh_var();
                let predicate = match op {
                    CmpOp::Eq => IcmpPredicate::Eq,
                    CmpOp::Ne => IcmpPredicate::Ne,
                    CmpOp::Lt => IcmpPredicate::Slt,
                    CmpOp::Le => IcmpPredicate::Sle,
                    CmpOp::Gt => IcmpPredicate::Sgt,
                    CmpOp::Ge => IcmpPredicate::Sge,
                };
                block.add_op(Operation {
                    results: vec![result.clone()],
                    kind: OperationKind::ICmp { predicate },
                    operands: vec![lhs_val, rhs_val],
                    result_types: vec![Type::new("i1")],
                    effects: vec![],
                    location: Self::loc(),
                    regions: vec![],
                });
                Ok(result)
            }
            Expr::Call { name, args } => {
                let mut arg_vals = Vec::new();
                for arg in args {
                    arg_vals.push(self.compile_expr(arg, block, env)?);
                }
                let result = self.fresh_var();
                block.add_op(Operation {
                    results: vec![result.clone()],
                    kind: OperationKind::Call {
                        callee: Symbol::new(name),
                    },
                    operands: arg_vals,
                    result_types: vec![Type::new("i64")],
                    effects: vec![],
                    location: Self::loc(),
                    regions: vec![],
                });
                Ok(result)
            }
        }
    }
}

/// Compile a MiniAIR source string into an AIR Module.
pub fn compile(source: &str) -> Result<Module, LangError> {
    let program = parse(source)?;
    compile_program(&program)
}

/// Compile a parsed Program into an AIR Module.
pub fn compile_program(program: &Program) -> Result<Module, LangError> {
    let mut compiler = Compiler::new();
    compiler.compile_program(program)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum LangError {
    #[error("lex error: {0}")]
    LexError(String),
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("compile error: {0}")]
    CompileError(String),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_function() {
        let src = r#"
            fn add(a: i64, b: i64) -> i64 {
                let c = a + b;
                return c;
            }
        "#;
        let prog = parse(src).unwrap();
        assert_eq!(prog.functions.len(), 1);
        assert_eq!(prog.functions[0].name, "add");
        assert_eq!(prog.functions[0].params.len(), 2);
        assert_eq!(prog.functions[0].ret_ty, Ty::I64);
        assert_eq!(prog.functions[0].body.len(), 2);
    }

    #[test]
    fn parse_if_else() {
        let src = r#"
            fn max(a: i64, b: i64) -> i64 {
                if a > b {
                    return a;
                } else {
                    return b;
                }
            }
        "#;
        let prog = parse(src).unwrap();
        assert_eq!(prog.functions[0].body.len(), 1);
        assert!(matches!(prog.functions[0].body[0], Stmt::If { .. }));
    }

    #[test]
    fn parse_multiple_functions() {
        let src = r#"
            fn square(x: i64) -> i64 {
                return x * x;
            }
            fn double(x: i64) -> i64 {
                return x + x;
            }
        "#;
        let prog = parse(src).unwrap();
        assert_eq!(prog.functions.len(), 2);
        assert_eq!(prog.functions[0].name, "square");
        assert_eq!(prog.functions[1].name, "double");
    }

    #[test]
    fn parse_nested_arithmetic() {
        let src = r#"
            fn calc(a: i64, b: i64) -> i64 {
                let c = (a + b) * (a - b);
                return c;
            }
        "#;
        let prog = parse(src).unwrap();
        assert_eq!(prog.functions[0].body.len(), 2);
    }

    #[test]
    fn parse_function_call() {
        let src = r#"
            fn helper(x: i64) -> i64 {
                return x + 1;
            }
            fn main() -> i64 {
                let r = helper(42);
                return r;
            }
        "#;
        let prog = parse(src).unwrap();
        assert_eq!(prog.functions.len(), 2);
        if let Stmt::Let { value, .. } = &prog.functions[1].body[0] {
            assert!(matches!(value, Expr::Call { .. }));
        } else {
            panic!("expected let");
        }
    }

    #[test]
    fn parse_with_comments() {
        let src = r#"
            // This is a comment
            fn id(x: i64) -> i64 {
                // return the argument
                return x;
            }
        "#;
        let prog = parse(src).unwrap();
        assert_eq!(prog.functions.len(), 1);
    }

    #[test]
    fn compile_add_function() {
        let src = r#"
            fn add(a: i64, b: i64) -> i64 {
                let c = a + b;
                return c;
            }
        "#;
        let module = compile(src).unwrap();
        assert_eq!(module.functions.len(), 1);
        let func = module.functions.get(&Symbol::new("add")).unwrap();
        assert_eq!(func.params.len(), 2);
        assert!(func.result.is_some());

        // Should have: const or add op + return
        let entry = func.entry_block().unwrap();
        assert!(entry.ops.len() >= 2); // at least add + return
    }

    #[test]
    fn compile_const_return() {
        let src = r#"
            fn forty_two() -> i64 {
                return 42;
            }
        "#;
        let module = compile(src).unwrap();
        let func = module.functions.get(&Symbol::new("forty_two")).unwrap();
        let entry = func.entry_block().unwrap();
        assert_eq!(entry.ops.len(), 2); // const + return
        assert!(matches!(entry.ops[0].kind, OperationKind::ConstI64(42)));
        assert!(matches!(entry.ops[1].kind, OperationKind::Return));
    }

    #[test]
    fn compile_if_else_produces_branches() {
        let src = r#"
            fn max(a: i64, b: i64) -> i64 {
                if a > b {
                    return a;
                } else {
                    return b;
                }
            }
        "#;
        let module = compile(src).unwrap();
        let func = module.functions.get(&Symbol::new("max")).unwrap();
        // Should have 3 blocks: entry, then, else.
        assert_eq!(func.blocks.len(), 3);
        // Entry block ends with CondBranch.
        let entry = &func.blocks[0];
        let terminator = entry.terminator().unwrap();
        assert!(matches!(terminator.kind, OperationKind::CondBranch { .. }));
    }

    #[test]
    fn compile_arithmetic_chain() {
        let src = r#"
            fn calc(x: i64) -> i64 {
                let a = x + 1;
                let b = a * 2;
                let c = b - 3;
                return c;
            }
        "#;
        let module = compile(src).unwrap();
        let func = module.functions.get(&Symbol::new("calc")).unwrap();
        let entry = func.entry_block().unwrap();
        // const(1) + add + const(2) + mul + const(3) + sub + return = 7 ops
        assert_eq!(entry.ops.len(), 7);
    }

    #[test]
    fn compile_call() {
        let src = r#"
            fn inc(x: i64) -> i64 {
                let y = x + 1;
                return y;
            }
            fn main() -> i64 {
                let r = inc(10);
                return r;
            }
        "#;
        let module = compile(src).unwrap();
        assert_eq!(module.functions.len(), 2);
        let main = module.functions.get(&Symbol::new("main")).unwrap();
        let entry = main.entry_block().unwrap();
        // const(10) + call + return = 3 ops
        assert_eq!(entry.ops.len(), 3);
        assert!(matches!(entry.ops[1].kind, OperationKind::Call { .. }));
    }

    #[test]
    fn lex_error_on_invalid_char() {
        let src = "fn foo() { @ }";
        let result = parse(src);
        assert!(result.is_err());
    }

    #[test]
    fn parse_error_missing_brace() {
        let src = "fn foo() {";
        let result = parse(src);
        // Should fail trying to parse the function body.
        assert!(result.is_err());
    }

    #[test]
    fn compile_undefined_var_error() {
        let src = r#"
            fn bad() -> i64 {
                return x;
            }
        "#;
        let result = compile(src);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("undefined variable"));
    }

    #[test]
    fn compile_void_return() {
        let src = r#"
            fn noop() {
                return;
            }
        "#;
        let module = compile(src).unwrap();
        let func = module.functions.get(&Symbol::new("noop")).unwrap();
        assert!(func.result.is_none());
    }
}
