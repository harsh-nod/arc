---
layout: default
title: Architecture
---

# Architecture

## Crate Map

ARC is organized as a Rust workspace with 22 crates. Each crate has a single responsibility.

```
arc/
  crates/
    arc_ir/          Core IR data structures (Module, Function, Block, Operation, Type)
    arc_syntax/      Parser (.air -> IR) and printer (IR -> .air)
    arc_verify/      SSA, type, effect, and authority verification
    arc_interp/      Reference interpreter with execution traces
    arc_pass/        Optimization passes (const fold, DCE, CSE, strength reduce)
    arc_lower/       Lowering passes (SCF, async, invoke, proof erasure)
    arc_codegen/     Code generation (low IR, isel, regalloc, x86-64, wasm)
    arc_targets/     Target descriptions (x86-64, wasm32 registers and opcodes)
    arc_object/      Object file emission (ELF)
    arc_effects/     Effect lattice and commutativity analysis
    arc_proof/       Proof obligation system (symbolic expressions, solvers)
    arc_security/    Information-flow analysis, taint tracking, sandboxing
    arc_types/       Type system (refinement types, dependent indices)
    arc_async/       Async runtime model (tasks, continuations, checkpoints)
    arc_runtime/     Runtime capability dispatch and approval policies
    arc_memory/      Memory model (provenance, alias analysis, borrow checking)
    arc_format/      Binary serialization format
    arc_pkg/         Package management
    arc_lang/        Language-level constructs
    arc_dialects/    Dialect registry
    arc_difftest/    Differential testing and fuzzing
    arc_cli/         Command-line interface
```

---

## Pipeline Stages

### 1. Parse

**Crate:** `arc_syntax`

The parser is a hand-written recursive descent parser that transforms `.air` text into the IR data model. It handles:

- Module structure and symbol tables
- Function signatures with parameters and return types
- Block labels and block arguments
- All operation kinds (arithmetic, control flow, capabilities, proofs, async)
- Region-based operations (if/else, loop)
- Effect and failure annotations on capabilities

The printer performs the inverse transformation (IR -> text), enabling parse-print round-trip testing.

### 2. Verify

**Crate:** `arc_verify`

Verification runs multiple passes over the IR:

1. **SSA check** -- Every value is defined exactly once, used after definition, dominance holds
2. **Type check** -- Operand types match operation requirements, result types are consistent
3. **Block structure** -- Every block has exactly one terminator, no unreachable blocks
4. **Resource check** -- Memory state threading is consistent (for memory operations)
5. **Region verification** -- Nested regions (if/loop bodies) inherit parent scope and verify independently

The verifier also integrates memory safety checking via `arc_memory::verify_memory_safety`.

### 3. Optimize

**Crate:** `arc_pass`

The pass manager runs optimization passes in sequence. Each pass transforms the module in-place.

| Pass | What it does |
|------|-------------|
| `const_fold` | Evaluates constant expressions at compile time |
| `dce` | Removes unused value definitions |
| `cse` | Eliminates redundant computations |
| `strength_reduce` | Replaces expensive ops with cheaper ones (mul by 2 -> shift) |
| `canonicalize` | Normalizes operation ordering for better CSE |

All passes recurse into region bodies (if/loop), so nested code is optimized too.

### 4. Lower

**Crate:** `arc_lower`

Lowering transforms high-level operations into forms suitable for code generation. Four passes run in sequence:

**4a. Structured Control Flow -> CFG** (`StructuredControlFlowLowering`)

Converts `arc.if`/`arc.loop`/`arc.yield` into flat blocks with `arc.cond_br`/`arc.br`:

```
arc.if %cond { ^then: ... } else { ^else: ... }
    |
    v
^entry:  arc.cond_br %cond, ^then, ^else
^then:   ... arc.br ^merge
^else:   ... arc.br ^merge
^merge:  ...
```

**4b. Async -> Sequential** (`AsyncLowering`)

Transforms async operations into sequential equivalents:

- `arc.spawn @f` -> `arc.call @f` (eager execution)
- `arc.await %handle` -> identity copy (add with zero)
- `arc.checkpoint` -> `arc.const 0` (no-op token)

**4c. Invoke -> Call** (`InvokeToCallLowering`)

Transforms capability invocations into regular function calls:

- `arc.invoke @cap` -> `arc.call @__cap_cap`
- `arc.require_approval` -> `arc.const 1` (granted token)
- Generates stub functions for each capability

**4d. Proof Erasure** (`ProofErasureLowering`)

Erases proof-carrying operations into runtime checks:

- `arc.assume` -> `arc.assert` + const proof token
- `arc.prove` -> `arc.assert` + const proof token
- `arc.refine` -> identity copy (add with zero)

Each lowering pass produces a **refinement record** documenting what properties are preserved (e.g., "same return value", "same effect trace").

### 5. Code Generation

**Crate:** `arc_codegen`

Code generation is a four-stage pipeline:

**5a. Low IR** (`low_ir`)

Lowers ARC operations to a machine-independent low-level IR:

| ARC Operation | Low IR |
|---------------|--------|
| `ConstI64(n)` | `LoadImm { dst, value }` |
| `Add/Sub/Mul/Div` | `Add/Sub/Mul/Div { dst, lhs, rhs }` |
| `ICmp` | `Cmp { dst, op, lhs, rhs }` |
| `Branch` | `Jump { target }` |
| `CondBranch` | `CondJump { cond, true_target, false_target }` |
| `Call` | `Call { dst, callee, args }` |
| `Return` | `Ret { value }` |
| `Alloc` | `StackAlloc { dst, size }` |
| `Load/Store` | `MemLoad/MemStore` |

**5b. Instruction Selection** (`isel`)

Maps low IR to target-specific instructions with virtual registers.

**5c. Register Allocation** (`regalloc`)

Assigns physical registers to virtual registers. Currently uses a linear scan approach.

**5d. Emission**

Two backends:

- **x86-64**: Emits machine code into an ELF `.o` object file (`arc_object`)
- **WebAssembly**: Emits a complete `.wasm` binary with type/function/export/code sections

---

## Security Analysis

**Crate:** `arc_security`

The security analysis runs independently of compilation. It performs:

### Information-Flow Analysis

Forward dataflow over SSA values. Each value carries:
- A **security level** (Public, Internal, Confidential, Secret)
- A set of **taint labels** (Trusted, UserInput, ThirdParty, ModelGenerated, NetworkInput)

The analysis propagates levels and taints through operations (result level = max of operand levels, result taints = union of operand taints) and checks for violations at capability invocations and external effects.

### Sandbox Enforcement

A `SandboxPolicy` restricts which capabilities a program may invoke and at what security level. Glob patterns are supported (`"network.*"` allows all network capabilities).

### Module Audit

Counts capability invocations, approval requests, external effects, and credential handling. Reports all violations found.

---

## Effect System

**Crate:** `arc_effects`

The effect lattice classifies operations into 18 categories. Effects support:

- **Ordering analysis**: Which operations can safely reorder?
- **Commutativity**: Reads commute with reads; writes block everything
- **Effect qualifiers**: deterministic, idempotent, reversible, etc.

---

## Proof System

**Crate:** `arc_proof`

The proof system supports:

- **Proof obligations**: BoundsCheck, Predicate, Refinement, Authority, Custom
- **Symbolic expressions**: i64 arithmetic with simplification
- **Linear arithmetic solver**: Discharges bounds and comparison predicates
- **Solver trait**: Extensible to external SMT backends

---

## Testing Infrastructure

**Crate:** `arc_difftest`

- **Conformance tests**: Validate that valid programs parse/verify and invalid programs are rejected
- **Differential tests**: Compare interpreter results across program variants
- **Fuzz testing**: Generate random programs and ensure parser/verifier don't panic
- **Integration tests**: End-to-end tests from `.air` files through the full pipeline

The project has a workspace-wide test suite covering unit, integration,
conformance, fuzz-smoke, optimization, lowering, codegen, and CLI behavior.

---

## Trace System

**Crate:** `arc_interp` (trace module)

The interpreter produces structured execution traces recording:

- Function entry/return with arguments and results
- Capability invocations with effects
- Approval requests with authority tokens
- Branch decisions with condition values
- Step counts for termination analysis

Traces serialize to JSON for storage, comparison, and replay.
