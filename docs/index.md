---
layout: default
title: ARC
---

# ARC: Agent Representation Compiler

ARC is a compiler intermediate representation for programs that interact with the outside world through **capabilities** - external actions like sending email, reading files, calling APIs, or writing files - with **authority**, **effects**, and **security** tracked by the compiler.

Unlike traditional IRs (MLIR, LLVM IR), ARC makes dangerous operations visible and verifiable:

```
arc.module @send_email_demo {
  arc.capability @email.send {
    inputs(%to: i64, %body: i64)
    outputs(%status: i64)
    effects [#arc.effect<network>, #arc.effect<irreversible>]
    failures [#arc.fail<delivery_error>]
  }

  arc.func @main() -> i64 {
  ^entry:
    %to   = arc.const 1 : i64
    %body = arc.const 2 : i64

    // This operation requires explicit approval before proceeding
    %auth = arc.require_approval %to, %body : !arc.auth<email.send>

    // The capability invocation is a first-class IR operation
    %status = arc.invoke @email.send(%to, %body)
    arc.return %status : i64
  }
}
```

In LLVM, `email.send` would be a function call indistinguishable from `strlen`. In ARC, the compiler **knows** it's irreversible, has network effects, can fail, and requires human approval.

---

## Why ARC?

AI systems increasingly act in the real world — booking flights, sending messages, modifying files, executing trades. Traditional compiler IRs treat all function calls equally. ARC doesn't.

**Capabilities are declared, not hidden.** Every external action has a name, typed inputs/outputs, declared effects, and enumerated failure modes. The compiler can verify that your program handles failures and obtains authority before acting.

**Effects are tracked, not guessed.** ARC's effect system distinguishes 18 categories (filesystem, network, financial, credential, irreversible, ...) with commutativity analysis. The optimizer knows which operations can safely reorder and which are anchored.

**Security is built in, not bolted on.** Information-flow analysis, taint tracking, and sandbox enforcement run at compile time. Confidential data can't leak to external sinks without explicit declassification. Tainted user input can't flow to approval operations.

**Proofs are first-class.** Bounds checks, preconditions, and authority requirements can be expressed as proof obligations that the compiler discharges statically or inserts as runtime checks.

---

## Quick Start

```bash
# Build from source
git clone https://github.com/harsh-nod/arc.git
cd arc
cargo build --release

# Parse and verify a program
arcc verify examples/hello.air

# Run a program through the interpreter
arcc run examples/hello.air
# Output: 10

# Compile to WebAssembly
arcc codegen examples/hello.air --target wasm32

# Run security analysis
arcc audit examples/send_email.air

# Optimize a program
arcc opt examples/hello.air --passes const_fold,dce
```

---

## Documentation

- [Getting Started](getting-started) — Install, write your first program, understand the CLI
- [Language Reference](language-reference) — Complete `.air` syntax and operation reference
- [Examples](examples) — Annotated programs showing capabilities, effects, branching, async
- [CLI Reference](cli) — Command-by-command reference for `arcc`
- [Authority and Effects](authority-effects) — Capability effects, approval tokens, and verifier rules
- [Architecture](architecture) — How the compiler pipeline works
- [Security](security) — Information-flow analysis, taint tracking, sandbox policies
- [Conformance](conformance) — Test suites, fuzz smoke tests, and release verification
- [Package and Binary Format](package-format) — Experimental module/package serialization
- [Release Guide](release) — Maintainer checklist for public releases

---

## The ARC Pipeline

```
  .air source
      |
   [ Parse ]          Recursive descent parser
      |
   [ Verify ]         SSA, types, effects, authority, proofs
      |
   [ Optimize ]       Constant fold, DCE, CSE, strength reduction
      |
   [ Lower ]          Structured CF -> flat CFG, async -> sequential,
      |               invoke -> call, proof erasure
      |
   [ Codegen ]        x86-64 ELF or WebAssembly
      |
   .o / .wasm
```

The interpreter can run programs directly after verification, producing execution traces with full effect and authority audit trails.

---

## What Makes ARC Different

| Feature | LLVM IR | MLIR | ARC |
|---------|---------|------|-----|
| Capabilities | No | No | Declared with effects, failures, authority |
| Effect tracking | No | Side-effect traits | 18-category lattice with commutativity |
| Authority / approval | No | No | `require_approval` + authority tokens |
| Information flow | No | No | 4-level classification + taint tracking |
| Proof obligations | No | No | First-class proof objects, discharge at compile time |
| Async / checkpoints | No | No | `spawn`, `await`, `checkpoint` as IR operations |
| Execution traces | No | No | Full audit trail with effect + authority events |

---

## Runtime Integration

ARC declares the **facts** about your program: what capabilities it uses, what effects it has, what authority it needs, and what proofs it carries. Runtime integrations can use those facts to dispatch capabilities, enforce authority, sandbox effects, and audit execution.

---

## Project Status

ARC is an early-stage research project. The core pipeline (parse, verify, interpret, optimize, lower, codegen) works end-to-end with the repository test suite. The unique features (capabilities, effects, authority, security analysis) are functional for the documented subset and covered by tests.

What's still evolving: dialect extensibility, SMT-backed proof solving, runtime capability dispatch, dependent types.

---

## License

Apache-2.0
