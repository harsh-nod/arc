---
layout: default
title: Language Reference
---

# ARC Language Reference

## Notation

- `@name` — symbol (module, function, capability name)
- `%name` — SSA value
- `^label` — block label
- `: type` — type annotation

---

## Module

A module is the top-level container. It holds functions, capabilities, and type declarations.

```
arc.module @name {
  // capabilities, functions
}
```

---

## Functions

```
arc.func @name(%arg1: type1, %arg2: type2) -> return_type {
^entry:
  // operations
}
```

Functions may have zero or more parameters and an optional return type. The entry block is the first block in the function body.

**Dependent index parameters** (for proof-carrying code):

```
arc.func @get[%n: index](%arr: !arc.array<i64, %n>, %idx: i64) -> i64 {
  // %n is a compile-time index parameter
}
```

---

## Blocks

Blocks are labeled sequences of operations. Every block must end with a terminator (`arc.return`, `arc.br`, `arc.cond_br`, `arc.yield`).

```
^block_name(%arg: type):
  // operations
  arc.return %result : type
```

Block arguments allow passing values along control-flow edges:

```
^entry:
  %x = arc.const 10 : i64
  arc.br ^next(%x)

^next(%val: i64):
  arc.return %val : i64
```

---

## Operations

### Constants

```
%x = arc.const 42 : i64     // 64-bit integer constant
%t = arc.const 1 : i1       // boolean true
%f = arc.const 0 : i1       // boolean false
```

### Arithmetic

```
%sum  = arc.add %a, %b : i64
%diff = arc.sub %a, %b : i64
%prod = arc.mul %a, %b : i64
%quot = arc.div %a, %b : i64
```

### Comparison

```
%r = arc.icmp <predicate> %a, %b : i1
```

Predicates: `eq`, `ne`, `slt` (signed less-than), `sle`, `sgt`, `sge`.

### Control Flow

```
// Unconditional branch
arc.br ^target(%arg1, %arg2)

// Conditional branch
arc.cond_br %cond, ^true_target(%args...), ^false_target(%args...)

// Return
arc.return %value : type
arc.return              // void return
```

### Structured Control Flow

```
// If-then-else (lowers to cond_br before codegen)
%result = arc.if %cond {
^then:
  %a = arc.const 10 : i64
  arc.yield %a
} else {
^else:
  %b = arc.const 20 : i64
  arc.yield %b
}

// Loop (lowers to branch/header/exit blocks)
arc.loop {
^body:
  // ... loop body ...
  arc.yield         // break (no args)
  arc.yield %val    // continue with updated value
}
```

### Function Calls

```
%result = arc.call @function_name(%arg1, %arg2)
```

---

## Capabilities

Capabilities declare external actions the program can perform. Every capability specifies its interface, effects, and failure modes.

```
arc.capability @name {
  inputs(%param1: type1, %param2: type2)
  outputs(%result: type)
  effects [#arc.effect<effect1>, #arc.effect<effect2>]
  failures [#arc.fail<failure_name>]
}
```

### Invoking Capabilities

```
%result = arc.invoke @capability_name(%arg1, %arg2)
```

The capability must be declared in the module. Argument and result types are
checked against the declaration.

### Authority

Before invoking a capability, programs must request matching approval:

```
%auth = arc.require_approval %arg1, %arg2 : !arc.auth<capability_name>
```

The verifier rejects an `arc.invoke @capability_name` unless a matching
`!arc.auth<capability_name>` token is available and dominates the invoke.
Runtime integrations can decide how approval is granted.

---

## Effect System

ARC tracks 18 effect categories:

| Effect | Description |
|--------|-------------|
| `pure` | No side effects |
| `memory.read` | Read from memory |
| `memory.write` | Write to memory |
| `allocate` | Allocate memory |
| `deallocate` | Free memory |
| `filesystem.read` | Read from filesystem |
| `filesystem.write` | Write to filesystem |
| `network` | Network communication |
| `database.read` | Database read |
| `database.write` | Database write |
| `ui` | User interface interaction |
| `llm` | Large language model invocation |
| `human.approval` | Requires human approval |
| `external_communication` | External messaging |
| `external_mutation` | External state mutation |
| `financial` | Financial transaction |
| `credential` | Credential handling |
| `physical` | Physical world action |
| `irreversible` | Cannot be undone |

Effects are declared on capabilities and operations:

```
effects [#arc.effect<network>, #arc.effect<irreversible>]
```

The optimizer uses effect commutativity to determine safe reorderings. Pure operations reorder freely. Operations with commuting effects (e.g., two reads) can reorder. Irreversible effects anchor evaluation order.

---

## Proof System

ARC supports proof-carrying code through three operations:

### Assume

Introduce a proof obligation that is assumed true:

```
%proof = arc.assume %condition : !arc.proof<predicate>
```

### Prove

Discharge a proof obligation (verified at compile time if possible):

```
%proof = arc.prove %condition : !arc.proof<predicate>
```

### Refine

Attach a proof to a value, creating a refined type:

```
%refined = arc.refine %value, %proof : !arc.refined<non_negative>
```

### Assert

Runtime check (fails if condition is false):

```
arc.assert %condition : i1
```

During lowering, `assume` and `prove` operations are erased to runtime assertions (`proof_erasure` pass). The verifier checks proof obligations statically where possible.

---

## Async Operations

ARC supports async task parallelism at the IR level:

```
// Spawn a task (returns a task handle)
%task = arc.spawn @worker_function(%arg1, %arg2)

// Await a task (blocks until completion, returns result)
%result = arc.await %task

// Checkpoint (marks a suspension point for continuation)
%token = arc.checkpoint "save_point"
```

During lowering, async operations are transformed to sequential equivalents (`async_to_sequential` pass):
- `spawn` becomes `call` (eager execution)
- `await` becomes identity copy
- `checkpoint` becomes `const 0`

---

## Types

### Built-in Types

| Type | Description |
|------|-------------|
| `i64` | 64-bit signed integer |
| `i32` | 32-bit signed integer |
| `i1` | Boolean |
| `index` | Compile-time index (for dependent parameters) |

### Pointer and Memory Types

| Type | Description |
|------|-------------|
| `!arc.ptr` | Opaque pointer |
| `!arc.ptr<T>` | Typed pointer |
| `!arc.mem` | Memory state (for memory threading) |
| `!arc.array<T, N>` | Array with element type and size |

### Special Types

| Type | Description |
|------|-------------|
| `!arc.proof<P>` | Proof object for predicate P |
| `!arc.refined<P>` | Value refined by predicate P |
| `!arc.auth<C>` | Authority token for capability C |
| `!arc.task` | Async task handle |

---

## Memory Operations

Memory operations use explicit state threading for safety:

```
// Allocate (takes memory state + size, returns new memory state + pointer)
%mem1, %ptr = arc.alloc %mem0, %size : -> (!arc.mem, !arc.ptr<i64>)

// Store (takes memory + pointer + value, returns new memory state)
%mem2 = arc.store %mem1, %ptr, %val : -> !arc.mem

// Load (takes memory + pointer, returns new memory state + loaded value)
%mem3, %loaded = arc.load %mem2, %ptr : -> (!arc.mem, i64)

// Indexed load with bounds proof
%elem = arc.load_elem %arr[%idx] requires %proof : -> i64
```

---

## Textual Syntax Grammar

```
module      ::= 'arc.module' SYMBOL '{' (capability | function)* '}'
capability  ::= 'arc.capability' SYMBOL '{' cap_body '}'
cap_body    ::= inputs outputs? effects? failures?
inputs      ::= 'inputs' '(' arg_list ')'
outputs     ::= 'outputs' '(' arg_list ')'
effects     ::= 'effects' '[' effect_list ']'
failures    ::= 'failures' '[' fail_list ']'
function    ::= 'arc.func' SYMBOL params? ('->' type)? '{' block+ '}'
params      ::= '(' arg_list ')'
block       ::= '^' IDENT block_args? ':' op+
block_args  ::= '(' arg_list ')'
arg_list    ::= arg (',' arg)*
arg         ::= VALUE ':' type
op          ::= (result_list '=')? op_name operands? (':' type_annot)?
result_list ::= VALUE (',' VALUE)*
operands    ::= VALUE (',' VALUE)*
SYMBOL      ::= '@' IDENT
VALUE       ::= '%' IDENT
```
